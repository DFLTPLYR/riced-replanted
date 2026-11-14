//! Current weather from Open-Meteo, refreshed independently of UI redraws.

use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use riced_extension_sdk::{Plugin, log};
use serde_json::{Value, json};

const ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
const FIELDS: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,weather_code,wind_speed_10m,wind_direction_10m";
const REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(60);
/// Locations tracked per process; `configure` beyond this drops the oldest.
const LIMIT: usize = 8;

/// Store key for a location, using Rust float formatting. Lua must look
/// locations up by coordinates instead of rebuilding this string, because
/// `tostring` renders `52.0` differently than Rust's `Display`.
fn key(latitude: f64, longitude: f64) -> String {
    format!("{latitude},{longitude}")
}

/// Reading and next fetch deadline per location.
type Readings = HashMap<String, (Value, Instant)>;

fn coordinates(args: &Value) -> Result<(f64, f64), String> {
    let args = args
        .as_array()
        .filter(|args| args.len() == 2)
        .ok_or("configure requires latitude and longitude")?;
    let latitude = args[0].as_f64().ok_or("latitude must be a number")?;
    let longitude = args[1].as_f64().ok_or("longitude must be a number")?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err("latitude must be -90..90 and longitude -180..180".into());
    }
    Ok((latitude, longitude))
}

fn current(agent: &ureq::Agent, endpoint: &str, location: (f64, f64)) -> Result<Value, String> {
    let url = format!(
        "{endpoint}?latitude={}&longitude={}&current={FIELDS}&timezone=auto",
        location.0, location.1
    );
    let bytes = agent
        .get(&url)
        .call()
        .map_err(|error| format!("Open-Meteo request failed: {error}"))?
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_vec()
        .map_err(|error| format!("Open-Meteo response failed: {error}"))?;
    let response: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Open-Meteo JSON: {error}"))?;
    if response["current"]["temperature_2m"].as_f64().is_none()
        || response["current"]["time"].as_str().is_none()
        || response["current_units"]["temperature_2m"].as_str() != Some("°C")
    {
        return Err("Open-Meteo response is missing current temperature, time or units".into());
    }
    Ok(json!({
        "ready": true,
        "latitude": location.0,
        "longitude": location.1,
        "current": response["current"],
        "current_units": response["current_units"],
        "timezone": response["timezone"],
        "updated_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
    }))
}

fn apply(state: &mut Value, result: Result<Value, String>) -> bool {
    match result {
        Ok(next) => {
            *state = next;
            true
        }
        Err(error) => {
            log::warn!(target: "openmeteo.fetch", "{error}");
            state["error"] = Value::String(error);
            false
        }
    }
}

/// One serve-loop pass: prune dropped locations, publish new ones as
/// placeholders, refresh every due reading, and mirror the most recently
/// configured location into the legacy fields. Returns the snapshot to
/// publish, or None when nothing changed.
fn tick(
    locations: &[(f64, f64)],
    readings: &mut Readings,
    state: &mut Value,
    agent: &ureq::Agent,
    endpoint: &str,
) -> Option<Value> {
    let mut dirty = false;
    let before = readings.len();
    readings.retain(|stored, _| locations.iter().any(|&(lat, lon)| key(lat, lon) == *stored));
    dirty |= readings.len() != before;
    for &(lat, lon) in locations {
        let key = key(lat, lon);
        if !readings.contains_key(&key) {
            readings.insert(
                key.clone(),
                (
                    json!({"ready": false, "latitude": lat, "longitude": lon}),
                    Instant::now(),
                ),
            );
            dirty = true;
        }
    }
    // New or pruned locations publish before any fetch so the most recent
    // location never shows another city's weather while it loads.
    let settled = !dirty;
    for &(lat, lon) in locations {
        let key = key(lat, lon);
        let (entry, deadline) = readings.get_mut(&key).unwrap();
        if settled && Instant::now() >= *deadline {
            let succeeded = apply(entry, current(agent, endpoint, (lat, lon)));
            *deadline = Instant::now() + if succeeded { REFRESH } else { RETRY };
            dirty = true;
        }
    }
    let legacy = match locations.first() {
        Some(&(lat, lon)) => readings[&key(lat, lon)].0.clone(),
        None => json!({"ready": false}),
    };
    dirty |= *state != legacy;
    *state = legacy;
    if !dirty {
        return None;
    }
    let mut published = state.clone();
    published["readings"] = json!(
        readings
            .iter()
            .map(|(stored, (entry, _))| (stored.clone(), entry.clone()))
            .collect::<BTreeMap<String, Value>>()
    );
    Some(published)
}

fn main() -> io::Result<()> {
    let configured = Arc::new(Mutex::new(Vec::<(f64, f64)>::new()));
    let tracked = Arc::clone(&configured);
    Plugin::new("openmeteo")
        .state([
            "ready",
            "latitude",
            "longitude",
            "current",
            "current_units",
            "timezone",
            "updated_at",
            "error",
            "readings",
        ])
        .method("configure", false, move |args| {
            let next = coordinates(args)?;
            let mut list = tracked
                .lock()
                .map_err(|_| "weather configuration lock poisoned")?;
            list.retain(|existing| *existing != next);
            list.insert(0, next);
            list.truncate(LIMIT);
            Ok(Value::Null)
        })
        .serve(move |context| {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .max_redirects(0)
                .build()
                .into();
            let mut readings = Readings::new();
            let mut state = json!({"ready": false});
            context.publish(state.clone());
            loop {
                let locations = match configured.lock() {
                    Ok(list) => list.clone(),
                    Err(error) => {
                        log::error!(target: "openmeteo.config", "{error}");
                        return;
                    }
                };
                if let Some(snapshot) =
                    tick(&locations, &mut readings, &mut state, &agent, ENDPOINT)
                {
                    context.publish(snapshot);
                }
                thread::sleep(Duration::from_secs(1));
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn coordinates_reject_invalid_arguments() {
        for args in [
            json!([]),
            json!([0]),
            json!([0, 0, 0]),
            json!(["0", 0]),
            json!([null, 0]),
            json!([91, 0]),
            json!([0, -181]),
        ] {
            assert!(coordinates(&args).is_err(), "{args}");
        }
        assert_eq!(coordinates(&json!([-90, 180])).unwrap(), (-90.0, 180.0));
        assert_eq!(coordinates(&json!([0, 0])).unwrap(), (0.0, 0.0));
    }

    #[test]
    fn fetch_handles_success_http_errors_invalid_and_oversized_responses() {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .into();
        let valid = json!({
            "current": {"temperature_2m": -2.5, "time": "2026-11-06T12:00", "weather_code": 3},
            "current_units": {"temperature_2m": "°C"}, "timezone": "Europe/Berlin"
        })
        .to_string();
        for (status, body, success) in [
            (200, valid, true),
            (429, "rate limited".into(), false),
            (200, "not JSON".into(), false),
            (200, "{}".into(), false),
            (200, " ".repeat(64 * 1024 + 1), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/v1/forecast", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert!(request.contains("latitude=52.52&longitude=13.41&current="));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
            let result = current(&agent, &endpoint, (52.52, 13.41));
            server.join().unwrap();
            assert_eq!(result.is_ok(), success, "{result:?}");
            if let Ok(weather) = result {
                assert_eq!(weather["current"]["temperature_2m"], -2.5);
                assert_eq!(weather["latitude"], 52.52);
            }
        }
    }

    #[test]
    fn failed_refresh_preserves_last_reading_and_success_clears_error() {
        let good = json!({"ready": true, "current": {"temperature_2m": 12.5}});
        let mut state = good.clone();
        assert!(!apply(&mut state, Err("offline".into())));
        assert_eq!(state["current"], good["current"]);
        assert_eq!(state["error"], "offline");
        assert!(apply(&mut state, Ok(good.clone())));
        assert_eq!(state, good);
    }

    #[test]
    fn coordinates_format_into_stable_store_keys() {
        assert_eq!(key(52.52, 13.41), "52.52,13.41");
        assert_eq!(key(52.0, 0.0), "52,0", "Rust Display, not Lua tostring");
        assert_eq!(key(-0.5, -180.0), "-0.5,-180");
    }

    #[test]
    fn tick_tracks_each_location_and_mirrors_the_latest() {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .into();
        let valid = json!({
            "current": {"temperature_2m": -2.5, "time": "2026-11-06T12:00", "weather_code": 3},
            "current_units": {"temperature_2m": "°C"}, "timezone": "Europe/Berlin"
        })
        .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/forecast", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = {
            let requests = Arc::clone(&requests);
            thread::spawn(move || {
                for (status, body) in [
                    (200, valid.clone()),
                    (200, valid),
                    (429, "rate limited".to_owned()),
                ] {
                    let Ok((mut stream, _)) = listener.accept() else {
                        break;
                    };
                    requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                            break;
                        }
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            })
        };
        let berlin = key(52.52, 13.41);
        let paris = key(48.85, 2.35);
        let mut readings = Readings::new();
        let mut state = json!({"ready": false});

        // Two locations appear as placeholders before the first fetch.
        let locations = [(52.52, 13.41), (48.85, 2.35)];
        let published = tick(&locations, &mut readings, &mut state, &agent, &endpoint).unwrap();
        assert_eq!(published["ready"], false);
        assert_eq!(published["latitude"], 52.52);
        assert_eq!(published["readings"].as_object().unwrap().len(), 2);
        assert_eq!(published["readings"][berlin.as_str()]["ready"], false);

        // Both fetch on the next pass; legacy fields mirror the first location.
        let published = tick(&locations, &mut readings, &mut state, &agent, &endpoint).unwrap();
        assert_eq!(published["ready"], true);
        assert_eq!(published["latitude"], 52.52);
        assert_eq!(
            published["readings"][berlin.as_str()]["current"]["temperature_2m"],
            -2.5
        );

        // An unchanged pass publishes nothing.
        assert!(tick(&locations, &mut readings, &mut state, &agent, &endpoint).is_none());

        // Reordering makes the most recent location the legacy mirror without refetching.
        let locations = [(48.85, 2.35), (52.52, 13.41)];
        let published = tick(&locations, &mut readings, &mut state, &agent, &endpoint).unwrap();
        assert_eq!(published["ready"], true);
        assert_eq!(published["latitude"], 48.85);
        assert_eq!(published["readings"].as_object().unwrap().len(), 2);

        // A failed refresh keeps one location's reading and sets only its error.
        readings.get_mut(&paris).unwrap().1 = Instant::now();
        let published = tick(&locations, &mut readings, &mut state, &agent, &endpoint).unwrap();
        assert!(published["readings"][paris.as_str()]["error"].is_string());
        assert_eq!(
            published["readings"][paris.as_str()]["current"]["temperature_2m"],
            -2.5
        );
        assert_eq!(
            published["readings"][berlin.as_str()]["current"]["temperature_2m"],
            -2.5
        );
        assert!(published["readings"][berlin.as_str()]["error"].is_null());

        server.join().unwrap();
        assert_eq!(
            requests.load(std::sync::atomic::Ordering::Relaxed),
            3,
            "placeholder, unchanged, and reorder passes must not refetch"
        );
    }
}
