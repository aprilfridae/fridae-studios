-- Finish an imported model. Paste into execute_luau (Edit) and edit the config.
-- Scales it to the size the Blender manifest gives, welds it, sets the pivot at
-- the base, makes MeshParts double sided, and rests it on the surface below.
local CONFIG = {
	path = "Workspace.TestLamp",        -- the inserted model
	name = "TestLamp",                  -- final name
	sizeStuds = { 2.4, 5.4, 2.4 },      -- size_studs from the manifest (x, y up, z)
	position = Vector3.new(0, 50, 0),   -- above the spot; it is dropped onto the surface
	yaw = 0,                            -- degrees around the vertical axis
	anchored = true,
}
local CHS = game:GetService("ChangeHistoryService")
local rec = CHS:TryBeginRecording("Place " .. CONFIG.name)
local inst = game
for seg in CONFIG.path:gmatch("[^%.]+") do
	if seg ~= "game" then inst = inst:FindFirstChild(seg) or error("not found: " .. seg) end
end
local model = inst
if not model:IsA("Model") then
	local wrap = Instance.new("Model")
	wrap.Parent = model.Parent
	model.Parent = wrap
	model = wrap
end
model.Name = CONFIG.name
local parts = {}
for _, d in ipairs(model:GetDescendants()) do
	if d:IsA("BasePart") then table.insert(parts, d) end
end
if #parts == 0 then error("the model has no parts (the asset may still be processing)") end
-- scale to the intended height
local _, size = model:GetBoundingBox()
local want = CONFIG.sizeStuds
local factor = want[2] / math.max(size.Y, 1e-4)
if math.abs(factor - 1) > 0.02 then model:ScaleTo(model:GetScale() * factor) end
for _, p in ipairs(parts) do
	p.Anchored = true
	if p:IsA("MeshPart") then p.DoubleSided = true end
end
-- pivot at the centre of the base
local cf, size2 = model:GetBoundingBox()
model.WorldPivot = CFrame.new(cf.Position - Vector3.new(0, size2.Y / 2, 0))
-- rest it on whatever is below the requested position
local params = RaycastParams.new()
params.FilterDescendantsInstances = { model }
params.FilterType = Enum.RaycastFilterType.Exclude
local hit = workspace:Raycast(CONFIG.position, Vector3.new(0, -2000, 0), params)
local ground = hit and hit.Position or CONFIG.position
model:PivotTo(CFrame.new(ground) * CFrame.Angles(0, math.rad(CONFIG.yaw), 0))
-- weld to a primary part so code can move it as one
local primary = model.PrimaryPart or parts[1]
for _, p in ipairs(parts) do
	if p.Size.Magnitude > primary.Size.Magnitude then primary = p end
end
model.PrimaryPart = primary
for _, p in ipairs(parts) do
	if p ~= primary and not p:FindFirstChildWhichIsA("WeldConstraint") then
		local w = Instance.new("WeldConstraint")
		w.Part0, w.Part1 = primary, p
		w.Parent = p
	end
	p.Anchored = CONFIG.anchored
end
local _, final = model:GetBoundingBox()
if rec then CHS:FinishRecording(rec, Enum.FinishRecordingOperation.Commit) end
return string.format("%s placed at (%.1f, %.1f, %.1f), size (%.2f, %.2f, %.2f), %d parts, scale factor %.3f, ground %s",
	model:GetFullName(), ground.X, ground.Y, ground.Z, final.X, final.Y, final.Z, #parts, factor, hit and hit.Instance:GetFullName() or "none found")
