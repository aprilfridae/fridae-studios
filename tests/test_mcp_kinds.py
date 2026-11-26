"""The tool server lists the Roblox tools only for a Roblox project (RBXKIT_ROBLOX, set by the app per chat)."""
import os
import unittest
from unittest import mock

import rbxkit_mcp


def listed(env):
    with mock.patch.dict(os.environ, env):
        res = rbxkit_mcp.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        hello = rbxkit_mcp.handle({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {}})
    return [t["name"] for t in res["tools"]], hello["instructions"]


class KindsTests(unittest.TestCase):
    def test_every_roblox_tool_is_a_real_tool(self):
        names = {t[0] for t in rbxkit_mcp.TOOLS}
        self.assertLessEqual(rbxkit_mcp.ROBLOX_TOOLS, names)

    def test_a_roblox_project_gets_every_tool(self):
        names, text = listed({"RBXKIT_ROBLOX": "1"})
        self.assertLessEqual(rbxkit_mcp.ROBLOX_TOOLS, set(names))
        self.assertIn("Roblox Studio", text)

    def test_outside_the_app_it_is_as_before(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("RBXKIT_ROBLOX", None)
            self.assertTrue(rbxkit_mcp.roblox_on())

    def test_any_other_project_gets_the_general_tools_and_words(self):
        names, text = listed({"RBXKIT_ROBLOX": "0"})
        self.assertFalse(set(names) & rbxkit_mcp.ROBLOX_TOOLS, names)
        for keep in ("rbx_vault", "rbx_skill", "rbx_guide", "rbx_board", "rbx_team", "rbx_image", "rbx_video", "rbx_stage"):
            self.assertIn(keep, names)
        self.assertNotIn("Roblox", text)
        with mock.patch.dict(os.environ, {"RBXKIT_ROBLOX": "0"}):
            out = rbxkit_mcp._reply({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                                     "params": {"name": "rbx_project", "arguments": {"action": "info"}}})
        self.assertIn("for Roblox projects", out["error"]["message"])


if __name__ == "__main__":
    unittest.main()
