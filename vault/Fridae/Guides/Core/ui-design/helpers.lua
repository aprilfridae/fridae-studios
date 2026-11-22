-- UI builder helpers. Paste at the top of an execute_luau (Edit) builder, then
-- build with them. Every helper sets the properties Roblox leaves at ugly
-- defaults (fill, border, font, text), so nothing ships with the default look.
-- Edit THEME to the game's palette before building; reuse the same table on
-- every screen. The defaults are deliberately quiet (neutrals, one muted
-- accent, thin outline, no gradients): the clean look under Taste in SKILL.md.
local THEME = {
	Panel = Color3.fromRGB(22, 22, 25),
	Header = Color3.fromRGB(30, 30, 34),
	Primary = Color3.fromRGB(78, 104, 160), -- the one accent: the main action
	Secondary = Color3.fromRGB(44, 44, 50),
	Danger = Color3.fromRGB(150, 70, 70),
	Text = Color3.fromRGB(236, 236, 240),
	Outline = Color3.fromRGB(62, 62, 70),
	Font = Enum.Font.BuilderSansMedium,
	TitleFont = Enum.Font.BuilderSansBold,
	Radius = 6, -- px
	Edge = 1, -- px outline; 0 for none
	Gradient = false, -- true only when the game's direction has glossy buttons
	WindowAlpha = 0.08, -- body see-through amount for big windows
}

local UI = { Theme = THEME }

function UI.new(class, props, parent)
	local inst = Instance.new(class)
	if inst:IsA("GuiObject") then
		inst.BorderSizePixel = 0
		inst.BackgroundTransparency = 1
	end
	for k, v in pairs(props or {}) do inst[k] = v end
	inst.Parent = parent
	return inst
end

-- A ScreenGui in StarterGui; replaces a previous one of the same name.
function UI.screen(name, displayOrder)
	local sg = game:GetService("StarterGui")
	local old = sg:FindFirstChild(name)
	if old then old:Destroy() end
	return UI.new("ScreenGui", {
		Name = name, ResetOnSpawn = false, DisplayOrder = displayOrder or 1,
		ZIndexBehavior = Enum.ZIndexBehavior.Sibling,
		ScreenInsets = Enum.ScreenInsets.CoreUISafeInsets,
	}, sg)
end

function UI.round(inst, px) return UI.new("UICorner", { CornerRadius = UDim.new(0, px or THEME.Radius) }, inst) end

function UI.outline(inst, px, color)
	return UI.new("UIStroke", {
		Thickness = px or THEME.Edge, Color = color or THEME.Outline,
		ApplyStrokeMode = Enum.ApplyStrokeMode.Border,
	}, inst)
end

-- A filled surface: card, well, HUD box.
function UI.panel(parent, props)
	props = props or {}
	local f = UI.new("Frame", {
		Name = props.Name or "Panel", Size = props.Size or UDim2.fromScale(1, 1),
		Position = props.Position or UDim2.new(), AnchorPoint = props.AnchorPoint or Vector2.zero,
		BackgroundColor3 = props.Color or THEME.Panel, BackgroundTransparency = props.Alpha or 0,
		LayoutOrder = props.LayoutOrder or 0,
	}, parent)
	UI.round(f, props.Radius)
	UI.outline(f)
	return f
end

-- Text that shrinks instead of clipping.
function UI.text(parent, text, props)
	props = props or {}
	local l = UI.new("TextLabel", {
		Name = props.Name or "Label", Text = text, Size = props.Size or UDim2.fromScale(1, 1),
		Position = props.Position or UDim2.new(), AnchorPoint = props.AnchorPoint or Vector2.zero,
		Font = props.Title and THEME.TitleFont or THEME.Font, TextColor3 = props.Color or THEME.Text,
		TextScaled = true, TextXAlignment = props.XAlign or Enum.TextXAlignment.Center,
		LayoutOrder = props.LayoutOrder or 0,
	}, parent)
	UI.new("UITextSizeConstraint", { MinTextSize = props.Min or 10, MaxTextSize = props.Max or 28 }, l)
	return l
end

-- An image shown as an object on the surface: no fill, aspect kept.
function UI.art(parent, image, props)
	props = props or {}
	local i = UI.new("ImageLabel", {
		Name = props.Name or "Art", Image = image, Size = props.Size or UDim2.fromScale(1, 1),
		Position = props.Position or UDim2.new(), AnchorPoint = props.AnchorPoint or Vector2.zero,
		ScaleType = Enum.ScaleType.Fit, LayoutOrder = props.LayoutOrder or 0,
	}, parent)
	UI.new("UIAspectRatioConstraint", { AspectRatio = props.Aspect or 1 }, i)
	return i
end

-- A button with a real base. Connect button.Activated in a LocalScript.
function UI.button(parent, text, color, props)
	props = props or {}
	local b = UI.new("TextButton", {
		Name = props.Name or (text:gsub("%W", "") .. "Button"), Text = "", AutoButtonColor = false,
		Size = props.Size or UDim2.fromOffset(160, 48), Position = props.Position or UDim2.new(),
		AnchorPoint = props.AnchorPoint or Vector2.new(0.5, 0.5),
		BackgroundColor3 = color or THEME.Primary, BackgroundTransparency = 0,
		LayoutOrder = props.LayoutOrder or 0,
	}, parent)
	UI.round(b)
	if THEME.Edge > 0 then UI.outline(b) end
	if THEME.Gradient then
		UI.new("UIGradient", {
			Rotation = 90,
			Color = ColorSequence.new(Color3.new(1, 1, 1), Color3.new(0.78, 0.78, 0.78)),
		}, b)
	end
	UI.new("UIScale", { Name = "ButtonScale" }, b)
	local label = UI.text(b, text, { Size = UDim2.fromScale(0.84, 0.7), Position = UDim2.fromScale(0.5, 0.5),
		AnchorPoint = Vector2.new(0.5, 0.5), Max = props.Max or 24 })
	return b, label
end

-- A centred window with a header, a close button and a see-through body.
function UI.window(gui, title, props)
	props = props or {}
	local root = UI.panel(gui, {
		Name = props.Name or (title:gsub("%W", "") .. "Window"),
		Size = props.Size or UDim2.fromScale(0.5, 0.6), Position = props.Position or UDim2.fromScale(0.5, 0.5),
		AnchorPoint = Vector2.new(0.5, 0.5), Alpha = THEME.WindowAlpha,
	})
	UI.new("UIAspectRatioConstraint", { AspectRatio = props.Aspect or 1.5 }, root)
	local header = UI.panel(root, { Name = "Header", Size = UDim2.fromScale(1, 0.14), Color = THEME.Header })
	UI.text(header, title, { Name = "Title", Title = true, Size = UDim2.fromScale(0.7, 0.7),
		Position = UDim2.fromScale(0.04, 0.15), XAlign = Enum.TextXAlignment.Left, Max = 36 })
	local close = UI.button(header, "X", THEME.Secondary, { Name = "Close", Size = UDim2.fromScale(0.09, 0.76),
		Position = UDim2.fromScale(0.94, 0.5) })
	UI.new("UIAspectRatioConstraint", { AspectRatio = 1 }, close)
	local body = UI.new("Frame", { Name = "Body", Size = UDim2.fromScale(0.94, 0.8),
		Position = UDim2.fromScale(0.5, 0.56), AnchorPoint = Vector2.new(0.5, 0.5) }, root)
	return { Root = root, Header = header, Body = body, Close = close }
end

function UI.grid(frame, cellSize, padding)
	return UI.new("UIGridLayout", { CellSize = cellSize, CellPadding = padding or UDim2.fromOffset(8, 8),
		SortOrder = Enum.SortOrder.LayoutOrder, HorizontalAlignment = Enum.HorizontalAlignment.Center }, frame)
end

function UI.list(frame, horizontal, padding)
	return UI.new("UIListLayout", { Padding = UDim.new(0, padding or 8), SortOrder = Enum.SortOrder.LayoutOrder,
		FillDirection = horizontal and Enum.FillDirection.Horizontal or Enum.FillDirection.Vertical }, frame)
end
