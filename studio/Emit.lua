-- VFX Emit: plays effect templates from attributes. rbxkit, readable on purpose.
-- Install as ReplicatedStorage.VFX.Emit (ModuleScript). Edit freely.
--
--   local Emit = require(game.ReplicatedStorage.VFX.Emit)
--   local h = Emit.play("Slam", CFrame.new(0, 3, 0))        -- clone, place, play, remove
--   local h = Emit.emit(sword.Blade)                          -- play emitters in place, restore after
--   h:setTimeScale(0.2)  h:freeze(0.08)  h:stop()
--   h:onStep(function(t, dt) end)  h:add("SlamDust", cf, 0.2)  h:onDone(function() end)
--   h.time  h.duration  h.finished  h.root
--
-- Attributes (the convention VFX editors share):
--   ParticleEmitter  EmitCount     burst this many (max 500)
--   ParticleEmitter  EmitDuration  emit at Rate for this many seconds
--   Beam, Trail      EmitDuration  enabled for this long
--   Lights           EmitDuration  on, then Brightness fades to 0 over this long
--   any of those     EmitDelay     seconds after the effect starts (absolute, never added to a parent's)
--   Model/Folder with Start and End parts:
--                    Duration, Easing, EasingDirection, Spin, EmitDelay
--                    Start moves to End's CFrame, Size, Transparency and Color;
--                    Spin is degrees about Start's own up axis
--   effect root      EffectDuration  when the effect ends (max 30 s)
local RunService = game:GetService("RunService")
local TweenService = game:GetService("TweenService")
local ContentProvider = game:GetService("ContentProvider")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local MAX_COUNT = 500
local MAX_DURATION = 30
local MIN_SCALE = 0.001

local Emit = {}
Emit._active = {}

local function note(fmt, ...)
	warn("VFXEmit: " .. string.format(fmt, ...))
end

local function attr(inst, name)
	local v = inst:GetAttribute(name)
	if v == nil then return nil end
	if type(v) ~= "number" then
		note("%s.%s should be a number, got %s", inst:GetFullName(), name, typeof(v))
		return nil
	end
	return v
end

local function isLight(inst)
	return inst:IsA("PointLight") or inst:IsA("SpotLight") or inst:IsA("SurfaceLight")
end

local function toCFrame(at)
	local t = typeof(at)
	if t == "CFrame" then return at end
	if t == "Vector3" then return CFrame.new(at) end
	if t == "Instance" then
		if at:IsA("Attachment") then return at.WorldCFrame end
		if at:IsA("BasePart") then return at.CFrame end
		if at:IsA("Model") then return at:GetPivot() end
	end
	return nil
end

local function resolve(effect)
	if type(effect) == "string" then
		local folder = ReplicatedStorage:FindFirstChild("VFX")
		local found = folder and folder:FindFirstChild(effect)
		if not found then error("VFXEmit: no effect named '" .. effect .. "' in ReplicatedStorage.VFX", 3) end
		return found
	end
	return effect
end

local function place(root, cf)
	if root:IsA("Model") then
		root:PivotTo(cf)
	elseif root:IsA("BasePart") then
		root.CFrame = cf
	else
		-- a Folder or Attachment-only container: move its parts by the same offset
		local first
		for _, d in ipairs(root:GetDescendants()) do
			if d:IsA("BasePart") then first = first or d end
		end
		if first then
			local delta = cf * first.CFrame:Inverse()
			for _, d in ipairs(root:GetDescendants()) do
				if d:IsA("BasePart") then d.CFrame = delta * d.CFrame end
			end
		end
	end
end

-- Collect everything the attributes drive.
local function collect(root)
	local items = {}
	local function visit(d)
		if d:IsA("ParticleEmitter") then
			local count, dur = attr(d, "EmitCount"), attr(d, "EmitDuration")
			if count or dur then
				if count and count > MAX_COUNT then
					note("%s EmitCount %d bounded to %d", d:GetFullName(), count, MAX_COUNT)
					count = MAX_COUNT
				end
				if dur and not count and d.Rate <= 0 then
					note("%s has EmitDuration but Rate 0: nothing to play", d:GetFullName())
				end
				table.insert(items, { kind = "particle", inst = d, delay = attr(d, "EmitDelay") or 0,
					count = count, duration = dur, baseScale = d.TimeScale, baseEnabled = d.Enabled })
			end
		elseif d:IsA("Beam") or d:IsA("Trail") then
			local dur, delay = attr(d, "EmitDuration"), attr(d, "EmitDelay")
			if dur or delay then
				table.insert(items, { kind = "switch", inst = d, delay = delay or 0, duration = dur or 0.3,
					baseEnabled = d.Enabled, baseLifetime = d:IsA("Trail") and d.Lifetime or nil })
			end
		elseif isLight(d) then
			local dur, delay = attr(d, "EmitDuration"), attr(d, "EmitDelay")
			if dur or delay then
				table.insert(items, { kind = "light", inst = d, delay = delay or 0, duration = dur or 0.2,
					baseEnabled = d.Enabled, baseBrightness = d.Brightness })
			end
		elseif d:IsA("Model") or d:IsA("Folder") then
			local s, e = d:FindFirstChild("Start"), d:FindFirstChild("End")
			if s and e and s:IsA("BasePart") and e:IsA("BasePart") then
				local okStyle, style = pcall(function() return Enum.EasingStyle[d:GetAttribute("Easing") or "Quad"] end)
				local okDir, dir = pcall(function() return Enum.EasingDirection[d:GetAttribute("EasingDirection") or "Out"] end)
				table.insert(items, { kind = "motion", inst = d, start = s, finish = e,
					delay = attr(d, "EmitDelay") or 0, duration = math.max(attr(d, "Duration") or 0.3, 0.001),
					style = okStyle and style or Enum.EasingStyle.Quad, dir = okDir and dir or Enum.EasingDirection.Out,
					spin = attr(d, "Spin") or 0,
					c0 = s.CFrame, s0 = s.Size, t0 = s.Transparency, k0 = s.Color,
					rel = s.CFrame:ToObjectSpace(e.CFrame), s1 = e.Size, t1 = e.Transparency, k1 = e.Color,
					endT = e.Transparency })
			end
		end
	end
	visit(root)
	for _, d in ipairs(root:GetDescendants()) do visit(d) end
	return items
end

local function totalDuration(root, items)
	local last, scheduled = 0, 0
	for _, it in ipairs(items) do
		if it.kind == "particle" then
			local life = it.inst.Lifetime.Max / math.max(it.baseScale, MIN_SCALE)
			last = math.max(last, it.delay + (it.duration or 0) + life)
			scheduled = math.max(scheduled, it.delay + (it.duration or 0))
		elseif it.kind == "switch" then
			last = math.max(last, it.delay + it.duration + (it.baseLifetime or 0))
			scheduled = math.max(scheduled, it.delay + it.duration)
		else
			last = math.max(last, it.delay + it.duration)
			scheduled = math.max(scheduled, it.delay + it.duration)
		end
	end
	local override = attr(root, "EffectDuration")
	if override then last = math.max(override, scheduled) end
	return math.min(last, MAX_DURATION)
end

function Emit.duration(effect)
	local root = resolve(effect)
	return totalDuration(root, collect(root))
end

-- List template mistakes as strings.
function Emit.check(effect)
	local root = resolve(effect)
	local out = {}
	local function add(fmt, ...) table.insert(out, string.format(fmt, ...)) end
	local parts, driven = 0, 0
	local all = root:GetDescendants()
	table.insert(all, root)
	for _, d in ipairs(all) do
		if d:IsA("BasePart") then
			parts += 1
			if not d.Anchored then add("%s is not anchored: it will fall", d.Name) end
			if d.CanCollide then add("%s has CanCollide on", d.Name) end
		elseif d:IsA("ParticleEmitter") then
			local count, dur = d:GetAttribute("EmitCount"), d:GetAttribute("EmitDuration")
			if count or dur then driven += 1 end
			if not count and not dur then
				if d.Rate > 0 and d.Enabled then
					add("%s has no EmitCount/EmitDuration: it streams forever and ignores slow motion", d.Name)
				else
					add("%s has no EmitCount/EmitDuration and Rate 0 or disabled: nothing to play", d.Name)
				end
			end
			if type(count) == "number" and count > MAX_COUNT then add("%s EmitCount %d is bounded to %d", d.Name, count, MAX_COUNT) end
			if dur and not count and d.Rate <= 0 then add("%s has EmitDuration but Rate 0", d.Name) end
			if d.Texture == "" then add("%s has no Texture", d.Name) end
			if d.Lifetime.Max > 10 then add("%s Lifetime %.1f s is very long", d.Name, d.Lifetime.Max) end
		elseif d:IsA("Beam") then
			if not d.Attachment0 or not d.Attachment1 then add("%s (Beam) is missing an attachment", d.Name) end
			if d:GetAttribute("EmitDuration") then driven += 1 end
		elseif d:IsA("Trail") then
			if not d.Attachment0 or not d.Attachment1 then add("%s (Trail) is missing an attachment", d.Name) end
			if d:GetAttribute("EmitDuration") then driven += 1 end
		elseif isLight(d) then
			if d:GetAttribute("EmitDuration") then driven += 1
			else add("%s (light) has no EmitDuration: it stays on", d.Name) end
		elseif (d:IsA("Model") or d:IsA("Folder")) and d:FindFirstChild("Start") and d:FindFirstChild("End") then
			driven += 1
		end
	end
	if driven == 0 then add("nothing carries EmitCount, EmitDuration or a Start/End pair: the effect does nothing") end
	return out
end

function Emit.preload(effect)
	local root = resolve(effect)
	local list = {}
	for _, d in ipairs(root:GetDescendants()) do
		if d:IsA("ParticleEmitter") or d:IsA("Beam") or d:IsA("Trail") or d:IsA("Decal") or d:IsA("MeshPart") then
			table.insert(list, d)
		end
	end
	if #list > 0 then pcall(function() ContentProvider:PreloadAsync(list) end) end
end

local Handle = {}
Handle.__index = Handle

local function applyScale(self)
	local scale = self._frozen and 0 or self._scale
	for _, it in ipairs(self._items) do
		if it.kind == "particle" then
			it.inst.TimeScale = it.baseScale * scale
		elseif it.kind == "switch" and it.baseLifetime and scale > MIN_SCALE then
			-- stretch trails so slow motion does not shorten them
			it.inst.Lifetime = math.min(it.baseLifetime / scale, 20)
		end
	end
	for _, child in ipairs(self._children) do
		child._parentScale = scale
		child._scale = child._ownScale * scale
		applyScale(child)
	end
end

function Handle:setTimeScale(scale)
	self._scale = math.clamp(scale, 0, 1) * (self._parentScale or 1)
	self._ownScale = math.clamp(scale, 0, 1)
	applyScale(self)
end

function Handle:freeze(seconds)
	self._frozen = true
	applyScale(self)
	task.delay(seconds, function()
		if self.finished then return end
		self._frozen = false
		applyScale(self)
	end)
end

function Handle:onStep(fn) table.insert(self._steps, fn) return self end
function Handle:onDone(fn)
	if self.finished then fn() else table.insert(self._done, fn) end
	return self
end

-- Start another effect on this effect's clock, `delay` effect-seconds from now.
function Handle:add(effect, at, delay)
	table.insert(self._pending, { effect = effect, at = at, when = self.time + (delay or 0) })
	self.duration = math.max(self.duration, self.time + (delay or 0) + Emit.duration(effect))
	return self
end

local function stepItem(self, it, t)
	if it.kind == "particle" then
		if not it.fired and t >= it.delay then
			it.fired = true
			if it.count then it.inst:Emit(it.count) end
			if it.duration then it.inst.Enabled = true end
		end
		if it.duration and not it.ended and t >= it.delay + it.duration then
			it.ended = true
			it.inst.Enabled = false
		end
	elseif it.kind == "switch" then
		if not it.fired and t >= it.delay then it.fired = true it.inst.Enabled = true end
		if not it.ended and t >= it.delay + it.duration then it.ended = true it.inst.Enabled = false end
	elseif it.kind == "light" then
		if not it.fired and t >= it.delay then it.fired = true it.inst.Enabled = true end
		if it.fired and not it.ended then
			local a = math.clamp((t - it.delay) / math.max(it.duration, 0.001), 0, 1)
			it.inst.Brightness = it.baseBrightness * (1 - a)
			if a >= 1 then it.ended = true it.inst.Enabled = false end
		end
	elseif it.kind == "motion" then
		if t >= it.delay and not it.ended then
			local raw = math.clamp((t - it.delay) / it.duration, 0, 1)
			local a = TweenService:GetValue(raw, it.style, it.dir)
			local base = it.c0 or it.start.CFrame
			it.start.CFrame = base:Lerp(base * it.rel, a) * CFrame.Angles(0, math.rad(it.spin * a), 0)
			it.start.Size = it.s0:Lerp(it.s1, a)
			it.start.Transparency = it.t0 + (it.t1 - it.t0) * a
			it.start.Color = it.k0:Lerp(it.k1, a)
			if raw >= 1 then it.ended = true end
		end
	end
end

function Handle:_step(dt)
	if self.finished then return end
	local scale = self._frozen and 0 or self._scale
	local edt = dt * scale
	self.time += edt
	local t = self.time
	if self._follow then
		local cf = toCFrame(self._follow)
		if cf then place(self.root, cf * self._offset) end
	end
	for _, it in ipairs(self._items) do stepItem(self, it, t) end
	for i = #self._pending, 1, -1 do
		local p = self._pending[i]
		if t >= p.when then
			table.remove(self._pending, i)
			local child = Emit.play(p.effect, p.at, { parent = self._parent, _driven = true })
			child._parentScale = scale
			child:setTimeScale(1)
			child.time = t - p.when
			table.insert(self._children, child)
		end
	end
	for _, child in ipairs(self._children) do child:_step(dt) end
	for _, fn in ipairs(self._steps) do
		local ok, err = pcall(fn, t, edt)
		if not ok then note("onStep error: %s", tostring(err)) end
	end
	if t >= self.duration and #self._pending == 0 then self:stop() end
end

function Handle:stop()
	if self.finished then return end
	self.finished = true
	if self._conn then self._conn:Disconnect() self._conn = nil end
	for _, child in ipairs(self._children) do child:stop() end
	if self._owned then
		if self.root then self.root:Destroy() end
	else
		for _, it in ipairs(self._items) do
			if it.kind == "particle" then
				it.inst.Enabled = it.baseEnabled
				it.inst.TimeScale = it.baseScale
			elseif it.kind == "switch" then
				it.inst.Enabled = it.baseEnabled
				if it.baseLifetime then it.inst.Lifetime = it.baseLifetime end
			elseif it.kind == "light" then
				it.inst.Enabled = it.baseEnabled
				it.inst.Brightness = it.baseBrightness
			elseif it.kind == "motion" then
				it.start.CFrame, it.start.Size, it.start.Transparency, it.start.Color = it.c0, it.s0, it.t0, it.k0
				it.finish.Transparency = it.endT
			end
		end
	end
	local idx = table.find(Emit._active, self)
	if idx then table.remove(Emit._active, idx) end
	for _, fn in ipairs(self._done) do
		local ok, err = pcall(fn)
		if not ok then note("onDone error: %s", tostring(err)) end
	end
end

local function start(root, owned, options)
	options = options or {}
	local items = collect(root)
	local self = setmetatable({
		root = root, time = 0, finished = false, duration = totalDuration(root, items),
		_items = items, _owned = owned, _scale = 1, _ownScale = 1, _frozen = false,
		_steps = {}, _done = {}, _pending = {}, _children = {},
		_parent = options.parent or workspace, _follow = options.follow, _offset = options.offset or CFrame.new(),
	}, Handle)
	for _, it in ipairs(items) do
		if it.kind == "particle" then
			it.inst.Enabled = false
		elseif it.kind == "switch" or it.kind == "light" then
			it.inst.Enabled = false
		elseif it.kind == "motion" then
			-- re-read the pose after placement; hide the End marker
			it.c0 = it.start.CFrame
			it.finish.Transparency = 1
		end
	end
	if #items == 0 then note("%s has nothing to play (no EmitCount, EmitDuration or Start/End)", root.Name) end
	if options.onDone then table.insert(self._done, options.onDone) end
	self:setTimeScale(options.timeScale or options.speed or 1)
	if not options._driven then
		table.insert(Emit._active, self)
		self._conn = RunService.Heartbeat:Connect(function(dt) self:_step(dt) end)
	end
	self:_step(0)
	return self
end

-- Clone a template, place it, play it, remove it when done.
function Emit.play(effect, at, options)
	options = options or {}
	local template = resolve(effect)
	local clone = template:Clone()
	local cf = toCFrame(at) or (options.follow and toCFrame(options.follow))
	if cf then place(clone, cf * (options.offset or CFrame.new())) end
	clone.Parent = options.parent or workspace
	return start(clone, true, options)
end

-- Play emitters where they already sit (on a weapon or character); restore after.
function Emit.emit(instance, options)
	return start(instance, false, options)
end

function Emit.stopAll()
	for i = #Emit._active, 1, -1 do Emit._active[i]:stop() end
end

return Emit
