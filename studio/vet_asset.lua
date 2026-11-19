-- Vet an inserted asset before trusting it. Run with execute_luau (Edit datamodel).
-- Reads only: nothing is changed. Set TARGET to the instance's path, then run.
-- Returns JSON: verdict "clean" | "review" | "dangerous", the reasons, every
-- script with its line count and flags, instance totals, and any welds or
-- constraints that reach outside the model.
--   dangerous: require(<asset id>), loadstring, obfuscated source
--   review:    getfenv/setfenv, HttpService requests, purchase prompts, TeleportService, InsertService,
--              DataStores, very long lines, scripts in odd places, links to outside
-- A "clean" verdict means none of these patterns were found, not that the code was understood: read
-- any script you are going to keep.
local TARGET = "Workspace.Model"   -- e.g. "Workspace.Free Tree" or "ServerStorage.Kit.Sword"

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

local root, missing = resolve(TARGET)
if not root then return "not found: no '" .. tostring(missing) .. "' on the way to " .. TARGET end
if root == game then return "set TARGET to the asset's path, not the whole game" end

-- { what to report, severity, patterns }
local CHECKS = {
	{ "require(<asset id>): loads code from outside the place", "dangerous",
		{ "require%s*%(?%s*%d%d%d+", "require%s*%(?%s*0[xX]%x+", "require%s*%(%s*tonumber" } },
	{ "loadstring: runs code built at run time", "dangerous", { "loadstring" } },
	{ "getfenv: reaches into the script environment", "review", { "getfenv" } },
	{ "setfenv: replaces the script environment", "review", { "setfenv" } },
	{ "HttpService requests: talks to outside servers", "review",
		{ "GetAsync", "PostAsync", "RequestAsync", "HttpEnabled" }, "HttpService" },   -- only when HttpService is named too
	{ "MarketplaceService prompt: asks players to buy something", "review",
		{ "PromptPurchase", "PromptProductPurchase", "PromptGamePassPurchase", "PromptSubscriptionPurchase",
			"PromptBundlePurchase", "PromptPremiumPurchase", "PromptBulkPurchase" } },
	{ "TeleportService: can move players to another place", "review", { "TeleportService" } },
	{ "InsertService: can load other assets", "review", { "InsertService", "LoadAsset" } },
	{ "DataStore use: reads or writes saved data", "review", { "DataStoreService", "GetDataStore", "GetOrderedDataStore", "GetGlobalDataStore" } },
}
local SPOOF = { Weld = true, Mesh = true, Part = true, Decal = true, Motor6D = true, Snap = true, Texture = true,
	TouchInterest = true, Attachment = true, Humanoid = true, Smoke = true, Fire = true, Sound = true, Joint = true }
local ODD_PARENTS = { "JointInstance", "Constraint", "WeldConstraint", "Attachment", "Decal", "DataModelMesh", "Sound",
	"ValueBase", "ParticleEmitter", "Light", "Fire", "Smoke", "Sparkles", "Humanoid", "Beam", "Trail" }
local HEX_RUN = string.rep("%x", 120)

local function obfuscation(src, longest)
	local why = {}
	local _, escapes = string.gsub(src, "\\%d%d?%d?", "")
	local _, hexEscapes = string.gsub(src, "\\x%x%x", "")
	if escapes + hexEscapes > 40 then table.insert(why, (escapes + hexEscapes) .. " escaped characters in strings") end
	if string.find(src, HEX_RUN) then table.insert(why, "a long run of hex digits") end
	if string.find(src, "[Il1][Il1][Il1][Il1][Il1][Il1][Il1][Il1][Il1]") then table.insert(why, "look-alike names such as IlIl1lI") end
	local _, chars = string.gsub(src, "string%.char", "")
	if chars > 8 then table.insert(why, chars .. " string.char calls") end
	if longest > 2000 then table.insert(why, "a line " .. longest .. " characters long") end
	return why
end

local rank = { clean = 0, review = 1, dangerous = 2 }
local verdict, reasons, seenReason = "clean", {}, {}
local function raise(level, reason)
	if rank[level] > rank[verdict] then verdict = level end
	if not seenReason[reason] then
		seenReason[reason] = true
		table.insert(reasons, level .. ": " .. reason)
	end
end

local function relative(inst)
	local names, cur = {}, inst
	while cur and cur ~= root do
		table.insert(names, 1, cur.Name)
		cur = cur.Parent
	end
	table.insert(names, 1, root.Name)
	return table.concat(names, ".")
end

local all = root:GetDescendants()
table.insert(all, 1, root)
local byClass, scripts, links = {}, {}, {}
local parts, unreadable = 0, 0
for n, d in ipairs(all) do
	byClass[d.ClassName] = (byClass[d.ClassName] or 0) + 1
	if d:IsA("BasePart") then parts += 1 end
	if d:IsA("LuaSourceContainer") then
		local ok, src = pcall(function() return d.Source end)
		local item = { path = relative(d), class = d.ClassName, flags = {} }
		if not ok or type(src) ~= "string" then
			unreadable += 1
			item.lines = 0
			table.insert(item.flags, "source could not be read")
			raise("review", "a script's source could not be read (" .. item.path .. ")")
		else
			local lines, longest = 0, 0
			for line in string.gmatch(src .. "\n", "([^\n]*)\n") do
				lines += 1
				if #line > longest then longest = #line end
			end
			if string.sub(src, -1) == "\n" or src == "" then lines -= 1 end
			item.lines, item.longest_line = lines, longest
			for _, check in ipairs(CHECKS) do
				local hit = false
				for _, pat in ipairs(check[4] and not string.find(src, check[4]) and {} or check[3]) do
					if string.find(src, pat) then hit = true break end
				end
				if hit then
					table.insert(item.flags, check[1])
					raise(check[2], check[1] .. " (" .. item.path .. ")")
				end
			end
			local why = obfuscation(src, longest)
			if #why > 0 then
				local text = "likely obfuscated: " .. table.concat(why, ", ")
				table.insert(item.flags, text)
				raise("dangerous", text .. " (" .. item.path .. ")")
			elseif longest > 500 then
				table.insert(item.flags, "very long line (" .. longest .. " characters)")
				raise("review", "very long line (" .. item.path .. ")")
			end
		end
		-- where it sits
		local parent = d.Parent
		local odd
		if d ~= root and parent then
			for _, class in ipairs(ODD_PARENTS) do
				if parent:IsA(class) then odd = "inside a " .. parent.ClassName break end
			end
		end
		local depth, cur = 0, d
		while cur and cur ~= root do depth += 1 cur = cur.Parent end
		if not odd and depth >= 8 then odd = depth .. " levels deep" end
		if not odd and (SPOOF[d.Name] or string.match(d.Name, "^%s*$") or #d.Name > 60) then
			odd = "named \"" .. string.sub(d.Name, 1, 30) .. "\", like a different kind of object"
		end
		if odd then
			table.insert(item.flags, "unusual place: " .. odd)
			raise("review", "script in an unusual place: " .. odd .. " (" .. item.path .. ")")
		end
		local okE, enabled = pcall(function() return d.Enabled end)
		if okE and enabled == false then item.disabled = true end
		table.insert(scripts, item)
	end
	-- joints and constraints that hold on to something outside
	for _, prop in ipairs({ "Part0", "Part1", "Attachment0", "Attachment1" }) do
		local ok, other = pcall(function() return (d :: any)[prop] end)
		if ok and typeof(other) == "Instance" and other ~= root and not other:IsDescendantOf(root) then
			table.insert(links, { from = relative(d), class = d.ClassName, property = prop, to = other:GetFullName() })
		end
	end
	if n % 4000 == 0 then task.wait() end
end
if #links > 0 then raise("review", #links .. " weld(s) or constraint(s) attached to things outside the model") end

local classes = {}
for class, count in pairs(byClass) do table.insert(classes, { class = class, count = count }) end
table.sort(classes, function(a, b) if a.count == b.count then return a.class < b.class end return a.count > b.count end)
while #classes > 15 do table.remove(classes) end
local shown = {}
table.sort(scripts, function(a, b) if #a.flags == #b.flags then return a.path < b.path end return #a.flags > #b.flags end)
for i = 1, math.min(#scripts, 200) do shown[i] = scripts[i] end
for i = #links, 41, -1 do links[i] = nil end

return game:GetService("HttpService"):JSONEncode({
	target = root:GetFullName(), verdict = verdict, reasons = reasons,
	totals = { instances = #all, parts = parts, scripts = #scripts, scripts_unreadable = unreadable, top_classes = classes },
	scripts = shown, scripts_not_shown = #scripts - #shown, outside_links = links,
})
