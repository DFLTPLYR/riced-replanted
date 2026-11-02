-- Subscriptions: timers, frames, and raw input.
-- Return one from app.subscription, or a list of them.

function iced.every(milliseconds)
	assert(
		type(milliseconds) == "number" and milliseconds >= 1 and milliseconds <= 86400000 and milliseconds % 1 == 0,
		"timer interval must be an integer from 1 to 86400000 ms"
	)
	return { every = milliseconds }
end

function iced.frames()
	return { type = "frames" }
end

function iced.on_key(pressed)
	assert(pressed == nil or type(pressed) == "boolean", "on_key pressed must be a boolean")
	return { type = "key", pressed = pressed }
end

function iced.on_mouse()
	return { type = "mouse" }
end
