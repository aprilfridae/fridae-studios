"""Where tests find real Roblox guide notes. The app no longer carries them (they come from the Fridae vault into
<home>/Knowledge/Roblox), so a test that reads real notes asks for them here: the checkout's vault/Roblox while it
is still there, then the checkout's Knowledge/Roblox (developer mode after Download), then FRIDAE_TEST_GUIDES."""
import os
import unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKIP = "No Roblox guides on this computer: run Download in developer mode, or set FRIDAE_TEST_GUIDES."


def _find():
    for d in (os.path.join(REPO, "vault", "Roblox"), os.path.join(REPO, "Knowledge", "Roblox"),
              os.environ.get("FRIDAE_TEST_GUIDES") or ""):
        if d and os.path.isdir(d):
            return d
    return None


GUIDES = _find()


def use_guides():
    """Point rbxvault.KB at the real guides for the rest of the run, or skip the caller's tests."""
    if GUIDES is None:
        raise unittest.SkipTest(SKIP)
    import rbxvault
    rbxvault.KB = GUIDES
