-- Fridae Studios link: the Studio side of the Fridae Studios app.
--
-- What it does, and all it does:
--   * tells the app what is selected in Studio, so "this" and "the selected part" mean something to the agent
--   * forwards errors and warnings from the output (edit mode and playtests) to the app
--   * keeps track of the undo steps the agent's work adds, and remembers every script's text at the
--     start of a turn, so the app can undo a whole turn (script edits are not in Studio's own undo)
--   * shows a small panel with what the app's chats are doing
-- It talks only to the Fridae Studios app on this computer (127.0.0.1), with a secret that the app
-- wrote into this file when it installed it. It never runs code sent to it: the app can only ask
-- for the fixed actions in `handle` below.

local TOKEN = "{{TOKEN}}"
local PORT = "{{PORT}}"
local VERSION = "{{VERSION}}"

local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local LogService = game:GetService("LogService")
local Selection = game:GetService("Selection")
local ChangeHistoryService = game:GetService("ChangeHistoryService")

local URL = "http://127.0.0.1:" .. PORT .. "/plugin/sync"
local DM = "Edit"
if RunService:IsRunning() then
	DM = RunService:IsServer() and "Server" or "Client"
end
local EDIT = DM == "Edit"

local alive = true
plugin.Unloading:Connect(function()
	alive = false
end)

-- ------------------------------------------------------------------ output
local logs = {}
local OWN = "[Fridae Studios]"
LogService.MessageOut:Connect(function(message, kind)
	local level = kind == Enum.MessageType.MessageError and "error" or kind == Enum.MessageType.MessageWarning and "warning" or nil
	if not level or string.find(message, OWN, 1, true) then
		return
	end
	if #logs >= 100 then
		table.remove(logs, 1)
	end
	table.insert(logs, { level = level, text = string.sub(message, 1, 600), t = os.time() })
end)

-- --------------------------------------------------------------- selection
local selectionDirty = true
local function describeSelection()
	local list, items = Selection:Get(), {}
	for i = 1, math.min(#list, 12) do
		local inst = list[i]
		local item = { path = inst:GetFullName(), class = inst.ClassName }
		if inst:IsA("BasePart") then
			local s, p = inst.Size, inst.Position
			item.detail = string.format("size %.1f x %.1f x %.1f at %.0f, %.0f, %.0f", s.X, s.Y, s.Z, p.X, p.Y, p.Z)
		elseif inst:IsA("Model") then
			local ok, size = pcall(function()
				return inst:GetExtentsSize()
			end)
			if ok then
				item.detail = string.format("about %.1f x %.1f x %.1f studs, %d parts inside", size.X, size.Y, size.Z, #inst:GetDescendants())
			end
		elseif inst:IsA("LuaSourceContainer") then
			item.detail = "script"
		elseif inst:IsA("GuiObject") then
			item.detail = string.format("%d x %d pixels on screen", inst.AbsoluteSize.X, inst.AbsoluteSize.Y)
		end
		table.insert(items, item)
	end
	return { count = #list, items = items }
end
if EDIT then
	Selection.SelectionChanged:Connect(function()
		selectionDirty = true
	end)
end

-- ---------------------------------------------------------- the undo stack
-- Each change the agent makes through Studio's own connector becomes one undo step named
-- "Assistant <n>". They are noted here as they appear, so a whole turn can be undone and
-- nothing else: undoing stops as soon as the next step is not one of these.
local steps = {}
local function now()
	return DateTime.now().UnixTimestampMillis / 1000
end
local reverting = false
local function topUndo()
	local can, name = ChangeHistoryService:GetCanUndo()
	return can and name or nil
end
if EDIT then
	ChangeHistoryService.OnRecordingFinished:Connect(function(name)
		if type(name) ~= "string" or string.sub(name, 1, 10) ~= "Assistant " then
			return
		end
		task.defer(function()
			if topUndo() == name and (steps[#steps] == nil or steps[#steps].name ~= name) then
				table.insert(steps, { name = name, t = now() })
				if #steps > 6000 then
					table.remove(steps, 1)
				end
			end
		end)
	end)
	ChangeHistoryService.OnUndo:Connect(function(name)
		if not reverting and steps[#steps] and steps[#steps].name == name then
			table.remove(steps)
		end
	end)
end

-- Script edits never reach Studio's undo history, so the text of every script is remembered when a
-- turn begins and put back on undo. Kept in memory only, for the last few turns.
local ScriptEditorService = game:GetService("ScriptEditorService")
local SCRIPT_ROOTS = { "Workspace", "ReplicatedStorage", "ReplicatedFirst", "ServerScriptService", "ServerStorage", "StarterGui", "StarterPack", "StarterPlayer", "Lighting", "SoundService" }
local snapshots = {}
local pinned = nil -- the snapshot from the start of an overnight run, kept until the next one
local function snapshot(since, pin)
	local sources = {}
	for _, name in ipairs(SCRIPT_ROOTS) do
		local root = game:FindService(name)
		if root then
			for _, d in ipairs(root:GetDescendants()) do
				if d:IsA("LuaSourceContainer") then
					local ok, src = pcall(function()
						return d.Source
					end)
					if ok then
						sources[d] = src
					end
				end
			end
		end
	end
	if pin then
		pinned = { since = since, sources = sources }
		return
	end
	table.insert(snapshots, { since = since, sources = sources })
	while #snapshots > 8 do
		table.remove(snapshots, 1)
	end
end

local function restoreScripts(since)
	-- the oldest snapshot taken at or after the moment asked for: the state when that turn began
	local snap
	local all = table.clone(snapshots)
	if pinned then
		table.insert(all, pinned)
	end
	for _, s in ipairs(all) do
		if s.since >= since and (not snap or s.since < snap.since) then
			snap = s
		end
	end
	if not snap then
		return 0, false
	end
	local restored = 0
	for inst, src in pairs(snap.sources) do
		if inst:IsDescendantOf(game) then
			local ok, now = pcall(function()
				return inst.Source
			end)
			if ok and now ~= src then
				local done = pcall(function()
					ScriptEditorService:UpdateSourceAsync(inst, function()
						return src
					end)
				end)
				if not done then
					done = pcall(function()
						inst.Source = src
					end)
				end
				if done then
					restored += 1
				end
			end
		end
	end
	return restored, true
end

local function revert(since)
	local undone, blocked = 0, nil
	reverting = true
	while undone < 6000 do
		local last = steps[#steps]
		if not last or last.t < since then
			break
		end
		local top = topUndo()
		if top ~= last.name then
			blocked = top or "nothing to undo"
			break
		end
		ChangeHistoryService:Undo()
		table.remove(steps)
		undone += 1
		task.wait() -- one frame, so the history is up to date before the next check (a night can be thousands of steps)
	end
	reverting = false
	local left = 0
	for _, s in ipairs(steps) do
		if s.t >= since then
			left += 1
		end
	end
	-- scripts go back only when the undo was not cut short, so the place is never left half way
	local scripts, had = 0, false
	if not blocked then
		scripts, had = restoreScripts(since)
	end
	return { undone = undone, blocked = blocked, left = left, scripts = scripts, had_snapshot = had }
end

-- the only things the app can ask for
local function handle(command)
	if command.kind == "begin" and EDIT then
		snapshot(tonumber(command.since) or now(), command.pin == true)
		return { ok = true }
	elseif command.kind == "revert" and EDIT then
		return revert(tonumber(command.since) or now())
	elseif command.kind == "count" and EDIT then
		local n = 0
		for _, s in ipairs(steps) do
			if s.t >= (tonumber(command.since) or 0) then
				n += 1
			end
		end
		return { steps = n }
	end
	return { error = "unknown request" }
end

-- -------------------------------------------------------------- the panel
local statusLine, agentLines, widget
if EDIT then
	local toolbar = plugin:CreateToolbar("Fridae Studios")
	local button = toolbar:CreateButton("Fridae", "Show what the Fridae Studios app is doing", "")
	button.ClickableWhenViewportHidden = true
	widget = plugin:CreateDockWidgetPluginGui("FridaeStudiosPanel", DockWidgetPluginGuiInfo.new(Enum.InitialDockState.Float, false, false, 280, 200, 220, 120))
	widget.Title = "Fridae Studios"
	widget.Name = "FridaeStudiosPanel"
	local frame = Instance.new("Frame")
	frame.Size = UDim2.fromScale(1, 1)
	frame.BackgroundColor3 = Color3.fromRGB(12, 12, 13)
	frame.BorderSizePixel = 0
	frame.Parent = widget
	local pad = Instance.new("UIPadding")
	pad.PaddingLeft, pad.PaddingRight, pad.PaddingTop, pad.PaddingBottom = UDim.new(0, 12), UDim.new(0, 12), UDim.new(0, 10), UDim.new(0, 10)
	pad.Parent = frame
	local function label(y, height, color, size)
		local l = Instance.new("TextLabel")
		l.BackgroundTransparency = 1
		l.Position = UDim2.fromOffset(0, y)
		l.Size = UDim2.new(1, 0, 0, height)
		l.Font = Enum.Font.Gotham
		l.TextSize = size
		l.TextColor3 = color
		l.TextXAlignment = Enum.TextXAlignment.Left
		l.TextYAlignment = Enum.TextYAlignment.Top
		l.TextWrapped = true
		l.Text = ""
		l.Parent = frame
		return l
	end
	statusLine = label(0, 34, Color3.fromRGB(236, 236, 238), 14)
	agentLines = label(40, 400, Color3.fromRGB(150, 150, 156), 13)
	statusLine.Text = "Looking for the app..."
	button.Click:Connect(function()
		widget.Enabled = not widget.Enabled
	end)
end

local function paint(connected, reply)
	if not statusLine then
		return
	end
	if not connected then
		statusLine.Text = "The Fridae Studios app is not open."
		agentLines.Text = "Open it and this connects by itself."
		return
	end
	local sel = #Selection:Get()
	statusLine.Text = "Connected to the app." .. (sel > 0 and ("  " .. sel .. " selected") or "")
	local lines = {}
	for _, a in ipairs(reply.agents or {}) do
		table.insert(lines, (a.busy and "> " or "  ") .. tostring(a.name) .. (a.busy and ("  -  " .. tostring(a.now or "working")) or ""))
	end
	agentLines.Text = #lines > 0 and table.concat(lines, "\n") or "No chats yet."
end

-- ------------------------------------------------------------------ the link
local results = {}
local function sync()
	local body = { dm = DM, version = VERSION, place = { name = game.Name, id = game.PlaceId }, logs = logs, results = results }
	if EDIT then
		if selectionDirty then
			selectionDirty = false
			body.selection = describeSelection()
		end
		body.undo_top = topUndo()
	end
	local sentLogs, sentResults = logs, results
	logs, results = {}, {}
	local ok, response = pcall(function()
		return HttpService:RequestAsync({
			Url = URL,
			Method = "POST",
			Headers = { ["Content-Type"] = "application/json", ["X-Fridae"] = TOKEN },
			Body = HttpService:JSONEncode(body),
		})
	end)
	if not ok or not response.Success then
		-- keep what was not delivered, so nothing is lost while the app is closed
		for i = #sentLogs, 1, -1 do
			table.insert(logs, 1, sentLogs[i])
		end
		while #logs > 100 do
			table.remove(logs, 1)
		end
		for i = #sentResults, 1, -1 do
			table.insert(results, 1, sentResults[i])
		end
		selectionDirty = true
		return false, nil
	end
	local decoded
	pcall(function()
		decoded = HttpService:JSONDecode(response.Body)
	end)
	return true, decoded or {}
end

task.spawn(function()
	local wasConnected = nil
	while alive do
		-- a playtest's server and client only speak up when they have something to report
		local quiet = not EDIT and #logs == 0
		local wait = 1
		if not quiet then
			local connected, reply = sync()
			if connected then
				for _, command in ipairs(reply.commands or {}) do
					local ok, result = pcall(handle, command)
					table.insert(results, { id = command.id, ok = ok, result = ok and result or { error = tostring(result) } })
				end
				if selectionDirty and EDIT then
					wait = 0.2
				end
			else
				wait = 4
			end
			if connected ~= wasConnected or EDIT then
				paint(connected, reply or {})
			end
			wasConnected = connected
		end
		task.wait(#results > 0 and 0.1 or wait)
	end
end)
