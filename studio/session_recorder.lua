-- Record what a player goes through in a playtest. Run with execute_luau on the CLIENT datamodel
-- during a playtest (it watches the local player, their leaderstats and their PlayerGui; it does
-- not work in Edit or on the Server). Reads only: it creates connections and one table in `shared`.
-- Set MODE and run:
--   "start"   begin recording (starts again from zero if already recording)
--   "report"  return JSON for the time recorded so far; recording carries on
--   "stop"    disconnect everything and forget the recording
-- The report: duration, time to the first change of any leaderstat, every leaderstat change with its
-- time, ScreenGuis and frames that became visible and when, deaths and respawns, distance travelled,
-- the longest spell with no movement and no stat change ("stuck"), the places visited (positions
-- rounded to a 16-stud grid, with seconds spent), and errors and warnings from the client's output.
-- Times are seconds since "start".
local MODE = "start"

local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local HttpService = game:GetService("HttpService")

local GRID = 16        -- studs: size of a "place visited" cell
local SAMPLE = 0.25    -- seconds between position samples
local MOVED = 0.4      -- studs in one sample that count as moving
local LIMIT = 400      -- most entries kept in any one list

local function stop()
	local S = shared.RbxSession
	if not S then return false end
	for _, conn in ipairs(S.conns) do pcall(function() conn:Disconnect() end) end
	shared.RbxSession = nil
	return true
end

if MODE == "stop" then
	return stop() and "stopped: connections closed and the recording forgotten" or "nothing was recording"
end

local function round(v, places)
	local m = 10 ^ (places or 2)
	return math.floor(v * m + 0.5) / m
end

if MODE == "report" then
	local S = shared.RbxSession
	if not S then return "not recording: run with MODE = \"start\" first" end
	local now = os.clock() - S.t0
	local idle = S.idle
	if now - S.lastActive > idle.seconds then idle = { seconds = now - S.lastActive, from = S.lastActive, to = now, at = S.lastCell } end
	local places = {}
	for cell, p in pairs(S.cells) do
		table.insert(places, { position = cell, first_seen = round(p.first), seconds = round(p.samples * SAMPLE, 1) })
	end
	table.sort(places, function(a, b) return a.first_seen < b.first_seen end)
	local dropped = math.max(0, #places - 60)
	while #places > 60 do table.remove(places) end
	local first = S.stats[1]
	return HttpService:JSONEncode({
		duration = round(now),
		time_to_first_stat_change = first and first.t or nil,   -- absent when no stat has changed yet
		stat_change_count = #S.stats,
		leaderstats_now = S.readStats(),
		stat_changes = S.stats, stat_changes_dropped = S.dropped.stats,
		ui_shown = S.ui, ui_shown_dropped = S.dropped.ui,
		deaths = S.deaths, respawns = S.respawns,
		distance_travelled = round(S.distance, 1), teleports = S.teleports,
		longest_stuck = { seconds = round(idle.seconds, 1), from = round(idle.from), to = round(idle.to), position = idle.at },
		places_visited = places, places_not_shown = dropped,
		errors = S.errors, warnings = S.warnings, output_dropped = S.dropped.output,
	})
end

if MODE ~= "start" then return "MODE must be \"start\", \"report\" or \"stop\"" end
if not RunService:IsRunning() or not RunService:IsClient() then
	return "run this on the Client datamodel during a playtest"
end
local player = Players.LocalPlayer
if not player then return "there is no LocalPlayer here: run this on the Client datamodel during a playtest" end
stop()

local S = { t0 = os.clock(), conns = {}, stats = {}, ui = {}, deaths = {}, respawns = {}, errors = {}, warnings = {},
	cells = {}, distance = 0, teleports = 0, lastActive = 0, lastCell = nil,
	idle = { seconds = 0, from = 0, to = 0 }, dropped = { stats = 0, ui = 0, output = 0 } }
shared.RbxSession = S

local function now() return round(os.clock() - S.t0) end
local function hook(signal, fn) table.insert(S.conns, signal:Connect(fn)) end
local function keep(list, item, counter)
	if #list < LIMIT then table.insert(list, item) else S.dropped[counter] += 1 end
end
local function active()
	local t = os.clock() - S.t0
	if t - S.lastActive > S.idle.seconds then S.idle = { seconds = t - S.lastActive, from = S.lastActive, to = t, at = S.lastCell } end
	S.lastActive = t
end

-- leaderstats
local function plain(v)
	local kind = type(v)
	if kind == "number" or kind == "string" or kind == "boolean" then return v end
	return tostring(v)
end
local watched = {}
local function watchStat(v)
	if watched[v] or not v:IsA("ValueBase") then return end
	watched[v] = true
	local last = plain(v.Value)
	hook(v.Changed, function(value)
		keep(S.stats, { t = now(), stat = v.Name, from = last, to = plain(value) }, "stats")
		last = plain(value)
		active()
	end)
end
local function watchFolder(folder)
	if folder.Name ~= "leaderstats" then return end
	for _, v in ipairs(folder:GetChildren()) do watchStat(v) end
	hook(folder.ChildAdded, watchStat)
end
for _, child in ipairs(player:GetChildren()) do watchFolder(child) end
hook(player.ChildAdded, watchFolder)
function S.readStats()
	local out = {}
	local folder = player:FindFirstChild("leaderstats")
	if folder then
		for _, v in ipairs(folder:GetChildren()) do
			if v:IsA("ValueBase") then out[v.Name] = plain(v.Value) end
		end
	end
	return out
end

-- ScreenGuis and the frames just inside them
local function guiPath(inst)
	local names, cur = {}, inst
	while cur and not cur:IsA("PlayerGui") do
		table.insert(names, 1, cur.Name)
		cur = cur.Parent
	end
	return table.concat(names, ".")
end
local function onScreen(inst)
	local cur = inst
	while cur and not cur:IsA("PlayerGui") do
		if cur:IsA("GuiObject") and not cur.Visible then return false end
		if cur:IsA("LayerCollector") and not cur.Enabled then return false end
		cur = cur.Parent
	end
	return cur ~= nil
end
local seenGui = {}
local function watchGui(inst)
	if seenGui[inst] then return end
	local prop
	if inst:IsA("ScreenGui") then
		prop = "Enabled"
	elseif inst:IsA("Frame") or inst:IsA("ScrollingFrame") or inst:IsA("CanvasGroup") then
		-- only panels near the top: a ScreenGui's children and grandchildren
		local parent = inst.Parent
		if not parent or not (parent:IsA("ScreenGui") or (parent.Parent and parent.Parent:IsA("ScreenGui"))) then return end
		prop = "Visible"
	else
		return
	end
	seenGui[inst] = true
	local shown = onScreen(inst)
	if shown and os.clock() - S.t0 > 0.5 then
		keep(S.ui, { t = now(), gui = guiPath(inst), class = inst.ClassName, how = "added" }, "ui")
	end
	hook(inst:GetPropertyChangedSignal(prop), function()
		local nowShown = onScreen(inst)
		if nowShown and not shown then keep(S.ui, { t = now(), gui = guiPath(inst), class = inst.ClassName, how = "shown" }, "ui") end
		shown = nowShown
	end)
end
local playerGui = player:FindFirstChildOfClass("PlayerGui")
if playerGui then
	for _, d in ipairs(playerGui:GetDescendants()) do watchGui(d) end
	hook(playerGui.DescendantAdded, watchGui)
	hook(playerGui.DescendantRemoving, function(d) seenGui[d] = nil end)
end

-- deaths and respawns
local function watchCharacter(char)
	task.spawn(function()
		local hum = char:WaitForChild("Humanoid", 10)
		if hum and shared.RbxSession == S then
			hook(hum.Died, function() table.insert(S.deaths, { t = now(), position = S.lastCell }) end)
		end
	end)
end
if player.Character then watchCharacter(player.Character) end
hook(player.CharacterAdded, function(char)
	table.insert(S.respawns, now())
	watchCharacter(char)
end)

-- movement
local lastPos, since = nil, 0
hook(RunService.Heartbeat, function(dt)
	since += dt
	if since < SAMPLE then return end
	since = 0
	local char = player.Character
	local rootPart = char and char:FindFirstChild("HumanoidRootPart")
	if not rootPart then lastPos = nil return end
	local pos = rootPart.Position
	local cell = string.format("%d, %d, %d", math.floor(pos.X / GRID + 0.5) * GRID, math.floor(pos.Y / GRID + 0.5) * GRID,
		math.floor(pos.Z / GRID + 0.5) * GRID)
	S.lastCell = cell
	local c = S.cells[cell]
	if not c then
		c = { first = os.clock() - S.t0, samples = 0 }
		S.cells[cell] = c
	end
	c.samples += 1
	if lastPos then
		local step = (pos - lastPos).Magnitude
		if step > 60 then
			S.teleports += 1   -- a respawn or a teleport, not walking
			active()
		elseif step > MOVED then
			S.distance += step
			active()
		end
	end
	lastPos = pos
end)

-- client output
hook(LogService.MessageOut, function(message, kind)
	if kind == Enum.MessageType.MessageError then
		keep(S.errors, { t = now(), text = string.sub(message, 1, 300) }, "output")
	elseif kind == Enum.MessageType.MessageWarning then
		keep(S.warnings, { t = now(), text = string.sub(message, 1, 300) }, "output")
	end
end)

return "recording " .. player.Name .. ": play, then run this again with MODE = \"report\" (and \"stop\" when finished)"
