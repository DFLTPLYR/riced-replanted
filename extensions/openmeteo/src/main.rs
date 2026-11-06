//! Current weather from Open-Meteo, refreshed independently of UI redraws.

use std::{
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

fn main() -> io::Result<()> {
    let location = Arc::new(Mutex::new(None));
    let configured = Arc::clone(&location);
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
        ])
        .method("configure", false, move |args| {
            let next = coordinates(args)?;
            *configured
                .lock()
                .map_err(|_| "weather configuration lock poisoned")? = Some(next);
            Ok(Value::Null)
        })
        .serve(move |context| {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .max_redirects(0)
                .build()
                .into();
            let mut previous = None;
            let mut deadline = Instant::now();
            let mut state = json!({"ready": false});
            context.publish(state.clone());
            loop {
                let next = match location.lock() {
                    Ok(location) => *location,
                    Err(error) => {
                        log::error!(target: "openmeteo.config", "{error}");
                        return;
                    }
                };
                if let Some(next) = next {
                    if previous != Some(next) {
                        // Never display the previous location's weather while a new one loads.
                        state = json!({"ready": false, "latitude": next.0, "longitude": next.1});
                        context.publish(state.clone());
                        previous = Some(next);
                        deadline = Instant::now();
                    }
                    if Instant::now() >= deadline {
                        let succeeded = apply(&mut state, current(&agent, ENDPOINT, next));
                        context.publish(state.clone());
                        deadline = Instant::now() + if succeeded { REFRESH } else { RETRY };
                    }
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
}
