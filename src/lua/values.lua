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
