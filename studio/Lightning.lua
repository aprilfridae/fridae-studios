-- VFX Lightning: bolts, chains and silhouette impact frames on an Emit handle's
-- clock. rbxkit, readable on purpose. Install as ReplicatedStorage.VFX.Lightning.
--
--   local h = Emit.play("ChainCast", caster.HumanoidRootPart)
--   Lightning.chain(h, { hand, enemyA, enemyB }, { hop = 0.06, hold = 1.2 })
--   Lightning.silhouette(h, { enemyA, enemyB }, { delay = 1.4 })
--
-- strike options (all optional): color, core, width (0.3), glow (width x 4),
-- haze (0.45), brightness (6), jag (0.12 of length), step (2.5 studs),
-- branches (2), flicker (0.04 s), delay (0), grow (0.06), hold (0.5),
-- fade (0.15), seed, parent. chain adds hop (0.05 s).
local Lightning = {}

local DEFAULT_COLOR = Color3.fromRGB(110, 140, 255)

local function toPosition(at)
	local t = typeof(at)
	if t == "Vector3" then return at end
	if t == "CFrame" then return at.Position end
	if t == "Instance" then
		if at:IsA("Attachment") then return at.WorldPosition end
		if at:IsA("BasePart") then return at.Position end
		if at:IsA("Model") then return at:GetPivot().Position end
	end
	error("Lightning: an end must be a Vector3, CFrame, Attachment, part or model", 3)
end

-- Just the shape: a jagged path from `from` to `to`.
function Lightning.points(from, to, segments, jag, seed)
	local a, b = toPosition(from), toPosition(to)
	local rng = Random.new(seed or os.clock() * 1000)
	local dir = b - a
	local length = dir.Magnitude
	if length < 0.001 then return { a, b } end
	local unit = dir / length
	local side = unit:Cross(math.abs(unit.Y) > 0.95 and Vector3.xAxis or Vector3.yAxis).Unit
	local up = unit:Cross(side).Unit
	local pts = { a }
	local wander = length * (jag or 0.12)
	local ox, oy = 0, 0
	for i = 1, segments - 1 do
		local f = i / segments
		-- a random walk that is pulled back to the line toward both ends
		ox += rng:NextNumber(-1, 1) * wander * 0.6
		oy += rng:NextNumber(-1, 1) * wander * 0.6
		local envelope = math.sin(f * math.pi)
		ox *= 0.75
		oy *= 0.75
		table.insert(pts, a + dir * f + (side * ox + up * oy) * envelope)
	end
	table.insert(pts, b)
	return pts
end

local function makeBeam(parent, a0, a1, color, width0, width1, transparency, brightness)
	local beam = Instance.new("Beam")
	beam.Attachment0, beam.Attachment1 = a0, a1
	beam.Color = ColorSequence.new(color)
	beam.Width0, beam.Width1 = width0, width1
	beam.Transparency = NumberSequence.new(transparency)
	beam.LightEmission = 1
	beam.LightInfluence = 0
	beam.Brightness = brightness
	beam.FaceCamera = true
	beam.Segments = 1
	beam.Texture = ""
	beam.Parent = parent
	return beam
end

function Lightning.strike(h, from, to, options)
	options = options or {}
	local color = options.color or DEFAULT_COLOR
	local core = options.core or Color3.new(1, 1, 1)
	local width = options.width or 0.3
	local glowWidth = options.glow or width * 4
	local haze = options.haze or 0.45
	local brightness = options.brightness or 6
	local jag = options.jag or 0.12
	local step = options.step or 2.5
	local branches = options.branches or 2
	local flicker = options.flicker or 0.04
	local delay = (options.delay or 0) + h.time
	local grow, hold, fade = options.grow or 0.06, options.hold or 0.5, options.fade or 0.15
	local seed = options.seed or math.floor(os.clock() * 1000) % 100000

	local holder = Instance.new("Part")
	holder.Name = "LightningBolt"
	holder.Anchored, holder.CanCollide, holder.CanQuery, holder.CanTouch = true, false, false, false
	holder.Transparency = 1
	holder.Size = Vector3.one * 0.05
	holder.CFrame = CFrame.new()
	holder.Parent = options.parent or workspace

	local bolt = { finished = false, points = {} }
	local length = (toPosition(to) - toPosition(from)).Magnitude
	local segments = math.clamp(math.ceil(length / step), 3, 48)
	local forkSegments = 8

	-- attachments and beams are made once and moved every flicker
	local function strand(count)
		local atts, beams = {}, {}
		for i = 1, count + 1 do
			local att = Instance.new("Attachment")
			att.Parent = holder
			atts[i] = att
		end
		for i = 1, count do
			local f0, f1 = (i - 1) / count, i / count
			local taper0, taper1 = 1 - 0.6 * f0, 1 - 0.6 * f1
			beams[i] = {
				glow = makeBeam(holder, atts[i], atts[i + 1], color, glowWidth * taper0, glowWidth * taper1, haze, brightness * 0.5),
				core = makeBeam(holder, atts[i], atts[i + 1], core, width * taper0, width * taper1, 0, brightness),
				baseHaze = haze,
			}
		end
		return { atts = atts, beams = beams, count = count }
	end

	local main = strand(segments)
	local forks = {}
	for i = 1, branches do forks[i] = strand(forkSegments) end

	local function layout(shapeSeed)
		local pts = Lightning.points(from, to, segments, jag, shapeSeed)
		bolt.points = pts
		for i, p in ipairs(pts) do main.atts[i].WorldPosition = p end
		local rng = Random.new(shapeSeed + 7)
		for _, fork in ipairs(forks) do
			local startIndex = rng:NextInteger(2, math.max(2, segments - 1))
			local origin = pts[startIndex]
			local along = (pts[#pts] - pts[1]).Unit
			local off = Vector3.new(rng:NextNumber(-1, 1), rng:NextNumber(-1, 1), rng:NextNumber(-1, 1))
			local target = origin + (along + off).Unit * length * rng:NextNumber(0.15, 0.3)
			local fpts = Lightning.points(origin, target, forkSegments, jag * 1.6, shapeSeed + startIndex)
			for i, p in ipairs(fpts) do fork.atts[i].WorldPosition = p end
		end
	end

	local function show(s, shown, alpha)
		for i, b in ipairs(s.beams) do
			local on = i <= shown
			b.glow.Enabled, b.core.Enabled = on, on
			if on then
				b.core.Transparency = NumberSequence.new(alpha)
				b.glow.Transparency = NumberSequence.new(b.baseHaze + (1 - b.baseHaze) * alpha)
			end
		end
	end

	function bolt.stop()
		if bolt.finished then return end
		bolt.finished = true
		holder:Destroy()
	end

	local total = grow + hold + fade
	if h.duration < delay + total then h.duration = delay + total end
	local lastShape = -1
	show(main, 0, 0)
	for _, f in ipairs(forks) do show(f, 0, 0) end

	h:onStep(function(t)
		if bolt.finished then return end
		local local_t = t - delay
		if local_t < 0 then return end
		if local_t >= total then bolt.stop() return end
		local shape = flicker > 0 and math.floor(local_t / flicker) or 0
		if shape ~= lastShape then
			lastShape = shape
			layout(seed + shape * 31)
		end
		local grown = grow > 0 and math.clamp(local_t / grow, 0, 1) or 1
		local alpha = local_t > grow + hold and math.clamp((local_t - grow - hold) / math.max(fade, 0.001), 0, 1) or 0
		show(main, math.ceil(grown * main.count), alpha)
		for _, f in ipairs(forks) do show(f, grown >= 1 and f.count or 0, math.min(1, alpha + 0.25)) end
	end)
	h:onDone(bolt.stop)
	return bolt
end

-- A strike per link: points[1] -> points[2] -> ... `hop` seconds apart.
function Lightning.chain(h, points, options)
	options = options or {}
	local hop = options.hop or 0.05
	local bolts = {}
	for i = 1, #points - 1 do
		local o = table.clone(options)
		o.delay = (options.delay or 0) + hop * (i - 1)
		-- every link holds until the whole chain has closed
		o.hold = (options.hold or 0.5) + hop * (#points - 1 - i)
		o.seed = options.seed and (options.seed + i * 977) or nil
		bolts[i] = Lightning.strike(h, points[i], points[i + 1], o)
	end
	return bolts
end

local DEFAULT_FRAMES = {
	{ duration = 0.07, backdrop = Color3.new(0, 0, 0), fill = Color3.fromRGB(90, 140, 255), outline = Color3.fromRGB(190, 215, 255) },
	{ duration = 0.07, backdrop = Color3.fromRGB(120, 235, 255), fill = Color3.new(0, 0, 0), outline = Color3.new(0, 0, 0) },
}

-- Impact frames: the world goes flat and only the targets show. Client only.
-- frames = { { duration, backdrop = Color3 or false, fill, outline }, ... }
function Lightning.silhouette(h, targets, options)
	options = options or {}
	local frames = options.frames or DEFAULT_FRAMES
	local delay = (options.delay or 0) + h.time
	local parent = options.parent or workspace
	local total = 0
	for _, f in ipairs(frames) do total += f.duration end
	if h.duration < delay + total then h.duration = delay + total end

	local made = {}
	local current = 0
	local card

	local function clear()
		for _, inst in ipairs(made) do inst:Destroy() end
		table.clear(made)
		card = nil
	end

	local function build(frame)
		clear()
		if frame.backdrop then
			-- a flat card just in front of the camera hides the world
			card = Instance.new("Part")
			card.Name = "SilhouetteCard"
			card.Anchored, card.CanCollide, card.CanQuery, card.CanTouch = true, false, false, false
			card.CastShadow = false
			card.Material = Enum.Material.Neon
			card.Color = frame.backdrop
			card.Size = Vector3.new(12, 12, 0.05)
			card.Parent = parent
			table.insert(made, card)
		end
		for _, target in ipairs(targets) do
			if typeof(target) == "Instance" and (target:IsA("Model") or target:IsA("BasePart")) then
				local hl = Instance.new("Highlight")
				hl.Adornee = target
				hl.DepthMode = Enum.HighlightDepthMode.AlwaysOnTop
				hl.FillColor = frame.fill or Color3.new(1, 1, 1)
				hl.FillTransparency = 0
				hl.OutlineColor = frame.outline or frame.fill or Color3.new(1, 1, 1)
				hl.OutlineTransparency = 0
				hl.Parent = parent
				table.insert(made, hl)
			end
		end
	end

	h:onStep(function(t)
		local local_t = t - delay
		if local_t < 0 then return end
		local acc, index = 0, 0
		for i, f in ipairs(frames) do
			if local_t < acc + f.duration then index = i break end
			acc += f.duration
		end
		if index ~= current then
			current = index
			if index == 0 then clear() else build(frames[index]) end
		end
		if card then
			local cam = workspace.CurrentCamera
			-- just in front of the camera, so it hides the whole world; the
			-- AlwaysOnTop highlights still draw over it
			card.CFrame = cam.CFrame * CFrame.new(0, 0, -1)
		end
	end)
	h:onDone(clear)
end

return Lightning
