-- UI audit. Paste this whole file as the `code` of execute_luau.
-- Edit datamodel: checks StarterGui. Client datamodel (playtest): checks PlayerGui.
-- Read-only: it changes nothing. Returns a text report.
local Players = game:GetService("Players")
local root = game:GetService("StarterGui")
if Players.LocalPlayer and Players.LocalPlayer:FindFirstChild("PlayerGui") then
	root = Players.LocalPlayer.PlayerGui
end

local DEFAULT_GREY = Color3.fromRGB(163, 162, 165)
local DEFAULT_TEXT = { Label = true, Button = true, TextLabel = true, TextButton = true, TextBox = true }
local MIN_TARGET = 44
local issues, counts = {}, {}

local function add(kind, inst, detail)
	counts[kind] = (counts[kind] or 0) + 1
	if #issues < 80 then
		table.insert(issues, string.format("%-15s %s%s", kind, inst:GetFullName(), detail and ("  (" .. detail .. ")") or ""))
	end
end

local function isLight(c)
	return c.R > 0.85 and c.G > 0.85 and c.B > 0.85
end

local function shown(g)
	local cur = g
	while cur and cur ~= root do
		if cur:IsA("GuiObject") and not cur.Visible then return false end
		if cur:IsA("ScreenGui") and not cur.Enabled then return false end
		cur = cur.Parent
	end
	return true
end

local function outside(inner, outer)
	local a, s = inner.AbsolutePosition, inner.AbsoluteSize
	local b, t = outer.AbsolutePosition, outer.AbsoluteSize
	local tol = 2
	return a.X < b.X - tol or a.Y < b.Y - tol or a.X + s.X > b.X + t.X + tol or a.Y + s.Y > b.Y + t.Y + tol
end

for _, g in ipairs(root:GetDescendants()) do
	if g:IsA("GuiObject") and shown(g) and not g:FindFirstAncestor("Chat") then
		local filled = g.BackgroundTransparency < 1
		local isText = g:IsA("TextLabel") or g:IsA("TextButton") or g:IsA("TextBox")
		local isImage = g:IsA("ImageLabel") or g:IsA("ImageButton")

		-- engine defaults left in place
		if filled and g.BackgroundColor3 == DEFAULT_GREY then add("default_look", g, "default grey fill") end
		if filled and g.BorderSizePixel > 0 then add("default_look", g, "border") end
		if isText then
			local ok, family = pcall(function() return g.FontFace.Family end)
			if ok and (family:find("SourceSansPro") or family:find("LegacyArial")) then add("default_look", g, "default font") end
			if DEFAULT_TEXT[g.Text] then add("default_look", g, "placeholder text '" .. g.Text .. "'") end
		end

		-- white or light plates
		if filled and isLight(g.BackgroundColor3) then
			if isText or isImage then
				add("white_box", g, "light fill behind text or art")
			elseif g.BackgroundTransparency > 0.05 then
				add("white_box", g, "see-through light fill")
			end
		end

		-- art
		if isImage and g.Image ~= "" then
			if filled then add("art_background", g) end
			if g:FindFirstChildWhichIsA("UIStroke") or g:FindFirstChildWhichIsA("UIGradient") then add("art_decorated", g) end
		end

		-- text fit
		if isText and g.Text ~= "" then
			if not g.TextFits then add("text_clipped", g, "'" .. g.Text:sub(1, 24) .. "'") end
			local host = g:FindFirstAncestorWhichIsA("GuiButton")
			if host and outside(g, host) then add("text_outside", g, "leaves " .. host.Name) end
			if g.TextScaled and not g:FindFirstChildWhichIsA("UITextSizeConstraint") then add("no_text_limit", g) end
			if g:FindFirstAncestorWhichIsA("CanvasGroup") then add("canvasgroup_text", g) end
		end

		-- touch targets
		if g:IsA("GuiButton") and g.Active then
			local s = g.AbsoluteSize
			if s.X > 0 and (s.X < MIN_TARGET or s.Y < MIN_TARGET) then
				add("small_target", g, string.format("%dx%d", s.X, s.Y))
			end
		end
	end
end

local kinds = {}
for k, n in pairs(counts) do table.insert(kinds, k .. "=" .. n) end
table.sort(kinds)
if #issues == 0 then return "UI audit (" .. root:GetFullName() .. "): clean" end
return "UI audit (" .. root:GetFullName() .. "): " .. table.concat(kinds, ", ") .. "\n" .. table.concat(issues, "\n")
