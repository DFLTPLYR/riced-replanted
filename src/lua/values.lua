-- Enum tables and value helpers shared by elements and layer settings.

iced = {}
iced.Layer = { background = "background", bottom = "bottom", top = "top", overlay = "overlay" }
iced.Extent = { fill = "fill" }
iced.Length = { fill = "fill", shrink = "shrink" }
iced.KeyboardInteractivity = { none = "none", on_demand = "on_demand", exclusive = "exclusive" }
iced.Output = { active = "active", last_output = "last_output" }
iced.Blur = { none = "none", full = "full" }

function iced.Size(width, height)
	assert(width ~= nil and height ~= nil, "size requires width and height")
	return { width = width, height = height }
end

function iced.Length.fill_portion(portion)
	assert(
		type(portion) == "number" and portion % 1 == 0 and portion >= 1 and portion <= 65535,
		"fill portion must be an integer from 1 to 65535"
	)
	return { fill_portion = portion }
end

function iced.opacity(color, percent)
	assert(type(color) == "string", "opacity color must be a string")
	assert(
		type(percent) == "number" and percent >= 0 and percent <= 100,
		"opacity must be a number from 0 to 100"
	)
	local hex = color:match("^(#%x%x%x%x%x%x)%x%x$") or color:match("^(#%x%x%x%x%x%x)$")
	assert(hex, "opacity color must be #RRGGBB or #RRGGBBAA")
	local alpha = math.floor(percent * 255 / 100 + 0.5)
	return string.format("%s%02x", hex, alpha)
end
