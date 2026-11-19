-- Delete one DataStore key so a test starts with a fresh player. Edit mode.
-- Needs "Enable Studio Access to API Services" in the place's Game Settings.
-- This deletes real saved data: confirm the store and key with the user first.
local STORE = "PlayerData"      -- DataStore name the game uses
local KEY = "Player_123456"     -- the exact key
local SCOPE = nil               -- optional scope
local DSS = game:GetService("DataStoreService")
local store = SCOPE and DSS:GetDataStore(STORE, SCOPE) or DSS:GetDataStore(STORE)
local okGet, before = pcall(function() return store:GetAsync(KEY) end)
if not okGet then return "could not read the key: " .. tostring(before) end
if before == nil then return "nothing stored under " .. KEY .. " in " .. STORE end
local ok, err = pcall(function() store:RemoveAsync(KEY) end)
return ok and ("removed " .. KEY .. " from " .. STORE) or ("failed: " .. tostring(err))
