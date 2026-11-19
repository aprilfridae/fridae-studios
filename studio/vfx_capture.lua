-- VFX capture helper. Paste once per Studio session with execute_luau
-- (Edit datamodel, or Client during a playtest). Needs ReplicatedStorage.VFX.Emit.
-- Then:  return shared.RbxVfx.start("Slam", Vector3.new(0, 3, 0))
--        return shared.RbxVfx.to(0.1)     -- then screen_capture; repeat per moment
--        return shared.RbxVfx.stop()
local VFX = game:GetService("ReplicatedStorage"):WaitForChild("VFX")
local K = shared.RbxVfx or {}
shared.RbxVfx = K

function K.stop()
	if K.Emit then pcall(K.Emit.stopAll) end
	if K.h and not K.h.finished then pcall(function() K.h:stop() end) end
	if K.folder then K.folder:Destroy() end
	local stray = workspace:FindFirstChild("RbxVfxPreview")
	if stray then stray:Destroy() end
	K.h, K.folder, K.Emit, K.begin = nil, nil, nil, nil
	return "stopped and cleaned up"
end

-- effect: template name in ReplicatedStorage.VFX or an instance.
-- at: Vector3, CFrame or instance. opts: { play = "luau returning a handle", size = studs, view = "iso" }
function K.start(effect, at, opts)
	opts = opts or {}
	K.stop()
	K.Emit = require(VFX.Emit:Clone()) -- a clone, so edits are not hidden by the require cache
	K.folder = Instance.new("Folder")
	K.folder.Name = "RbxVfxPreview"
	K.folder.Parent = workspace
	local tpl = type(effect) == "string" and VFX:FindFirstChild(effect) or effect
	if not tpl then K.stop() error("no effect '" .. tostring(effect) .. "' in ReplicatedStorage.VFX") end
	local cf = typeof(at) == "Vector3" and CFrame.new(at) or typeof(at) == "CFrame" and at
		or (at:IsA("Attachment") and at.WorldCFrame) or (at:IsA("Model") and at:GetPivot()) or at.CFrame
	local problems = K.Emit.check(tpl)
	K.Emit.preload(tpl)
	-- Nothing plays yet: the first to() call starts it, so particles are never
	-- frozen at birth.
	if opts.play then
		local fn, err = loadstring("local Emit, effect, at, parent = ...\n" .. opts.play)
		if not fn then K.stop() error("play code: " .. tostring(err)) end
		K.begin = function()
			local h = fn(K.Emit, tpl, cf, K.folder)
			if type(h) ~= "table" or not h.stop or not h.setTimeScale then K.stop() error("play code must return a handle with time, setTimeScale and stop") end
			return h
		end
	else
		K.begin = function() return K.Emit.play(tpl, cf, { parent = K.folder }) end
	end
	K.h = nil
	local duration = K.Emit.duration(tpl)
	-- camera suggestions for screen_capture
	local size = opts.size
	if not size then
		size = 8
		for _, d in ipairs(tpl:GetDescendants()) do
			if d:IsA("BasePart") then size = math.max(size, d.Size.Magnitude) end
			if d:IsA("ParticleEmitter") then size = math.max(size, d.Speed.Max * d.Lifetime.Max * 0.9 + 4) end
		end
	end
	local dirs = { iso = Vector3.new(0.7, 0.45, 0.7), front = Vector3.new(0, 0.1, -1), back = Vector3.new(0, 0.1, 1),
		left = Vector3.new(-1, 0.1, 0), right = Vector3.new(1, 0.1, 0), top = Vector3.new(0.01, 1, 0) }
	local dir = (dirs[opts.view or "iso"] or dirs.iso).Unit
	local dist = opts.distance or size * 1.4
	local p = cf.Position
	local cam = p + cf:VectorToWorldSpace(dir) * dist
	return string.format("duration=%.2f  look_at=[%.2f, %.2f, %.2f]  camera=[%.2f, %.2f, %.2f]  size=%.1f\ncheck: %s",
		duration, p.X, p.Y, p.Z, cam.X, cam.Y, cam.Z, size, #problems > 0 and table.concat(problems, " | ") or "clean")
end

-- Run the effect up to effect-time t and hold it there (slow = 0), or keep it
-- crawling for trails (slow = 0.04).
function K.to(t, slow)
	if not K.h then
		if not K.begin then return "nothing started: call start first" end
		K.h = K.begin()
		K.begin = nil
	end
	local h = K.h
	if h.finished then return string.format("ended (t=%.3f): nothing left to see", h.time) end
	h:setTimeScale(1)
	while not h.finished and h.time < t - 0.06 do task.wait() end
	h:setTimeScale(0.08)
	while not h.finished and h.time < t do task.wait() end
	if h.finished then return string.format("ended at t=%.3f", h.time) end
	h:setTimeScale(slow or 0)
	return string.format("holding at t=%.3f of %.2f", h.time, h.duration)
end

return "RbxVfx ready"
