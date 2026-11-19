-- Measure a sound: how long it is, how loud, and when the loud part happens. Run with execute_luau.
-- Where to run: Client during a playtest is the dependable choice, because PlaybackLoudness is
-- only reported on clients. In Edit the sound is played with SoundService:PlayLocalSound; if
-- Studio reports no loudness there, the result says loudness_measured = false and you should run
-- it on the Client instead. It does not work on the Server.
-- Set TARGET to a Sound's path or an asset id. The sound itself is never touched: a temporary
-- copy named RbxSoundProbe is played at a fixed volume and destroyed afterwards. You will hear it.
-- Returns JSON: time_length, is_loaded, played_seconds, peak_loudness (0 to 1000) and peak_at,
-- first_audible_at (seconds of silence at the start), and curve: the average loudness in 20 equal slices.
local TARGET = "rbxassetid://0"   -- "Workspace.Door.Open", 1234567890 or "rbxassetid://1234567890"
local MAX_SECONDS = 15            -- stop listening after this long
local VOLUME = 0.5                -- fixed, so different sounds can be compared
local AUDIBLE = 15                -- loudness above this counts as audible

local RunService = game:GetService("RunService")
local SoundService = game:GetService("SoundService")
local HttpService = game:GetService("HttpService")

local function resolve(path)
	local cur = game
	for name in string.gmatch(path, "[^%.]+") do
		if cur == game and name == "game" then continue end
		local nxt = cur:FindFirstChild(name)
		if not nxt and cur == game then
			pcall(function() nxt = game:FindService(name) end)
			if not nxt and string.lower(name) == "workspace" then nxt = workspace end
		end
		if not nxt then return nil, name end
		cur = nxt
	end
	return cur
end

local stray = SoundService:FindFirstChild("RbxSoundProbe")
if stray then stray:Destroy() end

local probe, source
local id = tonumber(TARGET) or (type(TARGET) == "string" and tonumber(string.match(TARGET, "^rbxassetid://(%d+)$")))
if id then
	if id == 0 then return "set TARGET to a Sound's path or an asset id" end
	probe = Instance.new("Sound")
	probe.SoundId = "rbxassetid://" .. id
	source = probe.SoundId
else
	local inst, missing = resolve(TARGET)
	if not inst then return "not found: no '" .. tostring(missing) .. "' on the way to " .. tostring(TARGET) end
	if not inst:IsA("Sound") then return inst:GetFullName() .. " is a " .. inst.ClassName .. ", not a Sound" end
	probe = Instance.new("Sound")
	probe.SoundId = inst.SoundId
	probe.PlaybackSpeed = inst.PlaybackSpeed
	source = inst:GetFullName() .. " (" .. inst.SoundId .. ")"
end
probe.Name = "RbxSoundProbe"
probe.Volume = VOLUME
probe.Looped = false
probe.Archivable = false
probe.Parent = SoundService

local function r(v, places)
	local m = 10 ^ (places or 2)
	return math.floor(v * m + 0.5) / m
end

local ok, result = pcall(function()
	local waited = os.clock()
	while not probe.IsLoaded and os.clock() - waited < 10 do task.wait(0.05) end
	local out = { source = source, is_loaded = probe.IsLoaded, time_length = r(probe.TimeLength, 3), volume = VOLUME,
		measured_in = not RunService:IsRunning() and "Edit" or (RunService:IsClient() and "Client" or "Server") }
	if not probe.IsLoaded then
		out.problem = "the sound did not load within 10 seconds: wrong id, not allowed in this place, or still being moderated"
		return out
	end
	if probe.TimeLength <= 0 then
		out.problem = "the sound loaded with no length"
		return out
	end
	local listen = math.min(probe.TimeLength / math.max(probe.PlaybackSpeed, 0.01), MAX_SECONDS)
	local samples = {}
	if RunService:IsRunning() then probe:Play() else SoundService:PlayLocalSound(probe) end
	local began = os.clock()
	while os.clock() - began < listen + 0.1 do
		RunService.Heartbeat:Wait()
		samples[#samples + 1] = { os.clock() - began, probe.PlaybackLoudness }
	end
	pcall(function() probe:Stop() end)
	local peak, peakAt, first = 0, 0, nil
	local sums, counts = table.create(20, 0), table.create(20, 0)
	for _, s in ipairs(samples) do
		local t, loud = s[1], s[2]
		if loud > peak then peak, peakAt = loud, t end
		if not first and loud > AUDIBLE then first = t end
		local slot = math.clamp(math.floor(t / listen * 20) + 1, 1, 20)
		sums[slot] += loud
		counts[slot] += 1
	end
	local curve = {}
	for i = 1, 20 do curve[i] = counts[i] > 0 and math.floor(sums[i] / counts[i] + 0.5) or 0 end
	out.played_seconds = r(listen)
	out.samples = #samples
	out.loudness_measured = peak > 0
	out.peak_loudness = r(peak, 0)
	out.peak_at = r(peakAt)
	out.first_audible_at = first and r(first) or nil
	out.curve = curve
	out.curve_slice_seconds = r(listen / 20, 3)
	if peak <= 0 then
		out.problem = "no loudness was reported here. Run it on the Client during a playtest."
	elseif not first then
		out.problem = "the sound never rose above the audible level (" .. AUDIBLE .. " of 1000): it is very quiet"
	elseif first > 0.15 then
		out.note = "about " .. r(first) .. " seconds of silence before the sound starts: it will feel late on a hit or a click"
	end
	if probe.TimeLength / math.max(probe.PlaybackSpeed, 0.01) > MAX_SECONDS then
		out.truncated = "only the first " .. MAX_SECONDS .. " seconds were measured"
	end
	return out
end)
probe:Destroy()
if not ok then return "sound_probe failed (the temporary sound was removed): " .. tostring(result) end
return HttpService:JSONEncode(result)
