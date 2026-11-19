-- Icon booth: show one model alone on a flat key colour, filling the screen, so
-- a screen_capture of it can be cut into a transparent icon with
-- `rbx.py icon`. Paste once per session with execute_luau (Edit). Then:
--   return shared.RbxIcon.show("Workspace.Sword", { view = "iso", ortho = true })
--   (screen_capture, no camera arguments)
--   return shared.RbxIcon.clear()
local K = shared.RbxIcon or {}
shared.RbxIcon = K
local KEY = Color3.fromRGB(255, 0, 255)
local VIEWS = {
	iso = Vector3.new(-0.7, 0.5, -0.7), iso_right = Vector3.new(0.7, 0.5, -0.7), front = Vector3.new(0, 0.1, -1),
	back = Vector3.new(0, 0.1, 1), left = Vector3.new(-1, 0.1, 0), right = Vector3.new(1, 0.1, 0), top = Vector3.new(0.01, 1, -0.01),
}

function K.clear()
	local host = game:GetService("StarterGui"):FindFirstChild("RbxIconBooth")
	if host then host:Destroy() end
	return "booth removed"
end

local function find(path)
	local inst = game
	for seg in path:gmatch("[^%.]+") do
		if seg ~= "game" then inst = inst:FindFirstChild(seg) or error("not found: " .. seg .. " in " .. path) end
	end
	return inst
end

-- opts: view (name or Vector3 direction the camera sits in), ortho (true = flat
-- isometric look), margin (1.1), yaw/pitch degrees instead of view,
-- ambient, light (Color3), lightDirection (Vector3)
function K.show(path, opts)
	opts = opts or {}
	K.clear()
	local source = type(path) == "string" and find(path) or path
	local gui = Instance.new("ScreenGui")
	gui.Name = "RbxIconBooth"
	gui.IgnoreGuiInset = true
	gui.DisplayOrder = 10000
	-- StarterGui, because screen_capture does not show CoreGui. clear() removes it.
	gui.Parent = game:GetService("StarterGui")
	local back = Instance.new("Frame")
	back.Size = UDim2.fromScale(1, 1)
	back.BackgroundColor3 = Color3.fromRGB(20, 20, 24)
	back.BorderSizePixel = 0
	back.Parent = gui
	local vp = Instance.new("ViewportFrame")
	vp.AnchorPoint = Vector2.new(0.5, 0.5)
	vp.Position = UDim2.fromScale(0.5, 0.5)
	vp.Size = UDim2.fromScale(0.96, 0.96)
	vp.SizeConstraint = Enum.SizeConstraint.RelativeYY
	vp.BackgroundColor3 = KEY
	vp.BorderSizePixel = 0
	vp.Ambient = opts.ambient or Color3.fromRGB(170, 170, 170)
	vp.LightColor = opts.light or Color3.fromRGB(255, 255, 255)
	vp.LightDirection = opts.lightDirection or Vector3.new(-0.5, -1, -0.6)
	vp.Parent = gui
	local world = Instance.new("WorldModel")
	world.Parent = vp
	local clone = source:Clone()
	if clone:IsA("BasePart") then
		local m = Instance.new("Model")
		clone.Parent = m
		clone = m
	end
	for _, d in ipairs(clone:GetDescendants()) do
		if d:IsA("BaseScript") or d:IsA("ParticleEmitter") or d:IsA("BillboardGui") then d:Destroy() end
	end
	clone.Parent = world
	local cf, size = clone:GetBoundingBox()
	local dir = VIEWS[opts.view or "iso"] or opts.view
	if opts.yaw or opts.pitch then
		local yaw, pitch = math.rad(opts.yaw or 0), math.rad(opts.pitch or 20)
		dir = Vector3.new(math.sin(yaw) * math.cos(pitch), math.sin(pitch), -math.cos(yaw) * math.cos(pitch))
	end
	dir = cf:VectorToWorldSpace(dir.Unit)
	local radius = size.Magnitude / 2
	local margin = opts.margin or 1.1
	local cam = Instance.new("Camera")
	local fov = opts.ortho and 5 or 30
	cam.FieldOfView = fov
	local dist = radius * margin / math.sin(math.rad(fov / 2))
	cam.CFrame = CFrame.lookAt(cf.Position + dir * dist, cf.Position)
	cam.Parent = vp
	vp.CurrentCamera = cam
	return string.format("booth showing %s (size %.1f, %.1f, %.1f). Capture now, then call clear().", source:GetFullName(), size.X, size.Y, size.Z)
end

return "RbxIcon ready"
