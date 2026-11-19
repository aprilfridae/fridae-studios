-- Performance snapshot. Run with execute_luau: Client during a playtest for real frame times and
-- memory; Edit also works (frame times are then Studio's editor, and memory includes Studio itself).
-- Reads only. Samples for SECONDS, then returns JSON: frame time avg / p95 / max in milliseconds
-- (Heartbeat, and RenderStepped where it can be connected), memory, instance count, counts of the
-- expensive classes in Workspace, unanchored parts, and `flags` for anything over a mobile budget.
--
-- Mobile budgets used for the flags. They are rules of thumb for a mid-range phone, not Roblox limits:
--   frame time     average over 18.5 ms (clearly under 60 fps; a screen-locked 60 reads about 16.7),
--                  or p95 over 33.3 ms (dips under 30 fps)
--   memory         over 1000 MB total on a real client (older phones close the app somewhere past this);
--                  not flagged inside Studio, where the total includes Studio itself
--   parts          over 20000 in Workspace, or over 10000 with StreamingEnabled off
--   unanchored     over 500 parts (each one is simulated)
--   unions         over 500
--   particles      over 100 emitters switched on, or their Rates adding up to more than 3000 a second
--   lights         over 100, or more than 10 casting shadows
--   beams + trails over 200
--   decals/textures over 1000
--   humanoids      over 50
local SECONDS = 5

local RunService = game:GetService("RunService")
local Stats = game:GetService("Stats")

-- frame times
local beats, renders = {}, {}
local conns = {}
table.insert(conns, RunService.Heartbeat:Connect(function(dt) beats[#beats + 1] = dt * 1000 end))
pcall(function()
	table.insert(conns, RunService.RenderStepped:Connect(function(dt) renders[#renders + 1] = dt * 1000 end))
end)

local t0 = os.clock()
while os.clock() - t0 < SECONDS do task.wait(0.1) end
for _, conn in ipairs(conns) do conn:Disconnect() end

-- counts, after the timing so that counting does not slow the frames being measured
local c = { parts = 0, plain_parts = 0, mesh_parts = 0, unions = 0, unanchored = 0, particle_emitters = 0,
	particle_emitters_on = 0, particle_rate = 0, lights = 0, shadow_lights = 0, beams = 0, trails = 0, decals = 0,
	textures = 0, humanoids = 0, surface_guis = 0, sounds = 0 }
local meshes, nMeshes = {}, 0
local world = workspace:GetDescendants()
for n, d in ipairs(world) do
	if d:IsA("BasePart") then
		c.parts += 1
		if not d.Anchored then c.unanchored += 1 end
		if d:IsA("MeshPart") then
			c.mesh_parts += 1
			if not meshes[d.MeshId] then meshes[d.MeshId] = true nMeshes += 1 end
		elseif d:IsA("PartOperation") then
			c.unions += 1
		elseif d.ClassName == "Part" then
			c.plain_parts += 1
		end
	elseif d:IsA("ParticleEmitter") then
		c.particle_emitters += 1
		if d.Enabled then
			c.particle_emitters_on += 1
			c.particle_rate += d.Rate
		end
	elseif d:IsA("Light") then
		c.lights += 1
		if d.Shadows and d.Enabled then c.shadow_lights += 1 end
	elseif d:IsA("Beam") then
		c.beams += 1
	elseif d:IsA("Trail") then
		c.trails += 1
	elseif d.ClassName == "Texture" then
		c.textures += 1
	elseif d.ClassName == "Decal" then
		c.decals += 1
	elseif d:IsA("Humanoid") then
		c.humanoids += 1
	elseif d:IsA("SurfaceGui") then
		c.surface_guis += 1
	elseif d:IsA("Sound") then
		c.sounds += 1
	end
	if n % 5000 == 0 then task.wait() end
end
c.unique_meshes = nMeshes
c.particle_rate = math.floor(c.particle_rate + 0.5)

local scripts = { Script = 0, LocalScript = 0, ModuleScript = 0 }
for _, name in ipairs({ "Workspace", "ReplicatedFirst", "ReplicatedStorage", "ServerScriptService", "ServerStorage",
	"StarterGui", "StarterPack", "StarterPlayer" }) do
	local ok, svc = pcall(function() return game:FindService(name) end)
	if ok and svc then
		for n, d in ipairs(svc:GetDescendants()) do
			if scripts[d.ClassName] then scripts[d.ClassName] += 1 end
			if n % 5000 == 0 then task.wait() end
		end
	end
end

local function summary(list)
	if #list == 0 then return nil end
	table.sort(list)
	local sum = 0
	for _, v in ipairs(list) do sum += v end
	local function r(v) return math.floor(v * 100 + 0.5) / 100 end
	local avg = sum / #list
	return { frames = #list, avg_ms = r(avg), p95_ms = r(list[math.max(1, math.ceil(#list * 0.95))]),
		max_ms = r(list[#list]), fps = r(1000 / avg) }
end

local memory = {}
pcall(function() memory.total_mb = math.floor(Stats:GetTotalMemoryUsageMb() + 0.5) end)
pcall(function()
	local tags = {}
	for _, tag in ipairs(Enum.DeveloperMemoryTag:GetEnumItems()) do
		local ok, mb = pcall(function() return Stats:GetMemoryUsageMbForTag(tag) end)
		if ok and mb and mb >= 1 then table.insert(tags, { tag = tag.Name, mb = math.floor(mb * 10 + 0.5) / 10 }) end
	end
	table.sort(tags, function(a, b) return a.mb > b.mb end)
	while #tags > 10 do table.remove(tags) end
	memory.largest = tags
end)

local instances
pcall(function() instances = Stats.InstanceCount end)
local running = RunService:IsRunning()
local where = not running and "Edit" or (RunService:IsClient() and "Client" or "Server")
local streaming = false
pcall(function() streaming = workspace.StreamingEnabled end)

local heartbeat, render = summary(beats), summary(renders)
local flags = {}
local function flag(text) table.insert(flags, text) end
local frame = render or heartbeat
if frame then
	if frame.avg_ms > 18.5 then flag(string.format("average frame %.1f ms (%.0f fps): under 60 fps", frame.avg_ms, frame.fps)) end
	if frame.p95_ms > 33.3 then flag(string.format("slowest 5%% of frames take over %.1f ms: dips under 30 fps", frame.p95_ms)) end
end
-- inside Studio the total includes Studio itself, so the flag would always fire: there it is only a note
local in_studio = game:GetService("RunService"):IsStudio()
if where == "Client" and not in_studio and memory.total_mb and memory.total_mb > 1000 then flag("memory " .. memory.total_mb .. " MB: over 1000 MB") end
if c.parts > 20000 then
	flag(c.parts .. " parts in Workspace: over 20000")
elseif c.parts > 10000 and not streaming then
	flag(c.parts .. " parts with StreamingEnabled off: over 10000")
end
if c.unanchored > 500 then flag(c.unanchored .. " unanchored parts: over 500") end
if c.unions > 500 then flag(c.unions .. " unions: over 500") end
if c.particle_emitters_on > 100 then flag(c.particle_emitters_on .. " particle emitters switched on: over 100") end
if c.particle_rate > 3000 then flag("particle rates add up to " .. c.particle_rate .. " a second: over 3000") end
if c.lights > 100 then flag(c.lights .. " lights: over 100") end
if c.shadow_lights > 10 then flag(c.shadow_lights .. " lights casting shadows: over 10") end
if c.beams + c.trails > 200 then flag((c.beams + c.trails) .. " beams and trails: over 200") end
if c.decals + c.textures > 1000 then flag((c.decals + c.textures) .. " decals and textures: over 1000") end
if c.humanoids > 50 then flag(c.humanoids .. " humanoids: over 50") end

local notes = {}
if where ~= "Client" then
	table.insert(notes, "Measured in " .. where .. ": frame times and memory are not what a player gets. Run on the Client during a playtest for those.")
end
if in_studio then
	table.insert(notes, "Memory was measured inside Studio, so the total includes Studio itself and is not a phone's figure. Compare the `largest` categories between runs instead (LuaHeap, Instances, Sounds, textures).")
end
table.insert(notes, "Frame times come from this computer, not a phone: a clean result here does not prove a phone keeps up.")

return game:GetService("HttpService"):JSONEncode({
	measured_in = where, seconds = SECONDS, heartbeat = heartbeat, render_stepped = render, memory = memory,
	instance_count = instances, workspace_descendants = #world, streaming_enabled = streaming,
	workspace = c, scripts = scripts, flags = flags, notes = notes,
})
