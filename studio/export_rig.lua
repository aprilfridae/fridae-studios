-- Export a rig as JSON for Blender. Paste into execute_luau (Edit).
-- RIG = a path string to a rig Model in the place, or "R15" / "R6" for a
-- standard dummy (made temporarily, then removed). Save the returned JSON as
-- work/rigs/<name>.json, then rbx.import_rig(path) in Blender.
local RIG = "R15"
local HttpService = game:GetService("HttpService")
local model, temp
if RIG == "R15" or RIG == "R6" then
	model = game:GetService("Players"):CreateHumanoidModelFromDescription(Instance.new("HumanoidDescription"),
		RIG == "R15" and Enum.HumanoidRigType.R15 or Enum.HumanoidRigType.R6)
	model:PivotTo(CFrame.new(0, 0, 0))
	temp = true
else
	model = game
	for seg in RIG:gmatch("[^%.]+") do
		if seg ~= "game" then model = model:FindFirstChild(seg) or error("not found: " .. seg) end
	end
end
local function r(n) return math.round(n * 10000) / 10000 end
local function cf(c)
	local t = { c:GetComponents() }
	for i, v in ipairs(t) do t[i] = r(v) end
	return t
end
local out = { name = temp and RIG or model.Name, parts = {}, joints = {} }
local rootPart = model:FindFirstChild("HumanoidRootPart") or model.PrimaryPart
local origin = rootPart and rootPart.CFrame or model:GetPivot()
for _, d in ipairs(model:GetDescendants()) do
	if d:IsA("BasePart") then
		table.insert(out.parts, { name = d.Name, size = { r(d.Size.X), r(d.Size.Y), r(d.Size.Z) }, cf = cf(origin:ToObjectSpace(d.CFrame)) })
	elseif d:IsA("Motor6D") and d.Part0 and d.Part1 then
		table.insert(out.joints, { name = d.Name, part0 = d.Part0.Name, part1 = d.Part1.Name, c0 = cf(d.C0), c1 = cf(d.C1) })
	elseif d:IsA("AnimationConstraint") and d.Attachment0 and d.Attachment1 then
		-- newer avatars: an AnimationConstraint between two attachments does a Motor6D's job
		table.insert(out.joints, { name = d.Name, part0 = d.Attachment0.Parent.Name, part1 = d.Attachment1.Parent.Name,
			c0 = cf(d.Attachment0.CFrame), c1 = cf(d.Attachment1.CFrame) })
	end
end
out.root = rootPart and rootPart.Name or nil
if temp then model:Destroy() end
return HttpService:JSONEncode(out)
