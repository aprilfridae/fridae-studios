"""Look up live Roblox experiences: what is out there, and how many are playing.

  rbx.py market search "pet simulator" --limit 10
  rbx.py market details 994732206 1686885941

Uses only Roblox's public web APIs, the ones the website itself calls for a
visitor who is not signed in. No login, no cookies, no account data; the only
thing sent is the search text or the universe ids.

  search   apis.roblox.com/search-api/omni-search      names, ids, players now, votes
  details  games.roblox.com/v1/games?universeIds=      visits, dates, genre, description
  votes    games.roblox.com/v1/games/votes?universeIds=

These are not documented, promised APIs: Roblox can change or close them. When
one stops answering, the tool says so plainly instead of guessing.
"""
import json, urllib.error, urllib.parse, urllib.request, uuid

from rbxpaths import KitError

AGENT = "rbxkit-market/1.0"
SEARCH = "https://apis.roblox.com/search-api/omni-search"
GAMES = "https://games.roblox.com/v1/games"
VOTES = "https://games.roblox.com/v1/games/votes"


class MarketError(KitError):
    pass


def _get(url, params):
    req = urllib.request.Request(url + "?" + urllib.parse.urlencode(params), headers={"User-Agent": AGENT, "Accept": "application/json"})
    host = urllib.parse.urlsplit(url).netloc
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        if e.code == 429:
            raise MarketError("Roblox is rate-limiting requests right now (HTTP 429 from %s). Wait a minute and try again." % host)
        if e.code in (401, 403):
            raise MarketError("Unavailable: %s now needs a signed-in account (HTTP %d). This tool only uses public endpoints, "
                              "so this lookup cannot be done." % (host, e.code))
        raise MarketError("Roblox answered HTTP %d from %s. The endpoint may have changed; try again later." % (e.code, host))
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        raise MarketError("Could not reach %s (%s). Check the internet connection and try again." % (host, getattr(e, "reason", e)))
    except ValueError:
        raise MarketError("%s did not answer with JSON. The endpoint may have changed." % host)


def _by_id(url, ids):
    """One of the games endpoints, asked about these universe ids: {universe id: its entry}."""
    return {g.get("id"): g for g in _get(url, {"universeIds": ",".join(str(i) for i in ids)}).get("data") or []}


def _ids(values):
    out = []
    for v in values or []:
        try:
            n = int(str(v).strip())
        except ValueError:
            raise MarketError("\"%s\" is not a universe id. Universe ids are whole numbers (search returns them)." % v)
        if n > 0 and n not in out:
            out.append(n)
    return out


def details(universe_ids):
    """Full rows for these universe ids, most players first."""
    ids = _ids(universe_ids)
    if not ids:
        raise MarketError("Give at least one universe id.")
    if len(ids) > 50:
        raise MarketError("At most 50 universe ids at a time.")
    games = _by_id(GAMES, ids)
    try:
        votes, votes_note = _by_id(VOTES, ids), None
    except MarketError as e:
        votes, votes_note = {}, str(e)
    rows = []
    for i in ids:
        v = votes.get(i) or {}
        if games.get(i):
            rows.append(_row(i, games[i], v.get("upVotes"), v.get("downVotes")))
    rows.sort(key=lambda r: -(r["playing"] or 0))
    return {"games": rows, "not_found": [i for i in ids if i not in games], "votes_note": votes_note}


def _row(uid, g, up, down, fallback=None):
    fallback = fallback or {}
    desc = " ".join(str(g.get("description") or fallback.get("description") or "").split())
    genre = g.get("genre_l1") or g.get("genre") or None
    if genre and g.get("genre_l2"):
        genre += " / " + g["genre_l2"]
    total = (up or 0) + (down or 0)
    creator = g.get("creator") or {}
    return {"name": g.get("name") or fallback.get("name"), "universe_id": uid,
            "place_id": g.get("rootPlaceId") or fallback.get("rootPlaceId"),
            "playing": g.get("playing") if g.get("playing") is not None else fallback.get("playerCount"),
            "visits": g.get("visits"), "favorites": g.get("favoritedCount"),
            "like_ratio": round(up / total, 4) if total else None, "up_votes": up, "down_votes": down,
            "created": (g.get("created") or "")[:10] or None, "updated": (g.get("updated") or "")[:10] or None,
            "genre": genre if genre and genre != "All" else None, "max_players": g.get("maxPlayers"),
            "creator": creator.get("name"), "description": desc[:200] + ("..." if len(desc) > 200 else "")}


def search(query, limit=10):
    """Experiences matching the query, most players first."""
    query = " ".join(str(query or "").split())
    if not query:
        raise MarketError("Give 'query': the words to search for.")
    if len(query) > 100:
        raise MarketError("The query is too long (100 characters at most).")
    limit = max(1, min(20, int(limit or 10)))
    data = _get(SEARCH, {"searchQuery": query, "sessionId": str(uuid.uuid4()), "pageType": "all"})
    found = {}  # universe id: its search entry, in Roblox's order, each experience once
    for group in data.get("searchResults") or []:
        if group.get("contentGroupType") == "Game":
            for c in group.get("contents") or []:
                if c.get("universeId") and not c.get("isSponsored"):
                    found.setdefault(c["universeId"], c)
    if not found:
        return {"query": query, "games": [], "note": "No experiences matched."}
    found = list(found.values())[:40]
    note = None
    try:
        extra = _by_id(GAMES, [c["universeId"] for c in found])
    except MarketError as e:
        extra, note = {}, "Visits, dates, genre and descriptions are missing: " + str(e)
    rows = [_row(c["universeId"], extra.get(c["universeId"]) or {}, c.get("totalUpVotes"), c.get("totalDownVotes"), c) for c in found]
    for i, r in enumerate(rows, 1):
        r["search_rank"] = i  # Roblox's own order, before sorting by players
    rows.sort(key=lambda r: -(r["playing"] or 0))
    return {"query": query, "games": rows[:limit], "matched": len(found), "note": note}


def _count(n):
    if n is None:
        return "-"
    for cut, unit in ((1e9, "B"), (1e6, "M"), (1e3, "K")):
        if n >= cut:
            return "%.1f%s" % (n / cut, unit)
    return str(n)


def report(res, title):
    rows = res["games"]
    lines = [title]
    if rows:
        lines.append("")
        for i, r in enumerate(rows, 1):
            lines.append("%d. %s" % (i, r["name"]))
            lines.append("   playing now %s | visits %s | likes %s | created %s | updated %s%s" % (
                _count(r["playing"]), _count(r["visits"]), "%.0f%%" % (r["like_ratio"] * 100) if r["like_ratio"] is not None else "-",
                r["created"] or "-", r["updated"] or "-", " | genre %s" % r["genre"] if r["genre"] else ""))
            lines.append("   universe id %s | place id %s%s%s" % (r["universe_id"], r["place_id"], " | by %s" % r["creator"] if r["creator"] else "",
                                                                " | Roblox search rank %d" % r["search_rank"] if r.get("search_rank") else ""))
            if r["description"]:
                lines.append("   " + r["description"])
    for k in ("note", "votes_note"):
        if res.get(k):
            lines.append(res[k])
    if res.get("not_found"):
        lines.append("No experience found for universe id(s): " + ", ".join(str(i) for i in res["not_found"]))
    if rows and rows[0].get("search_rank"):
        lines += ["", "Sorted by players, not by how well they match: Roblox's search also returns loosely related popular "
                      "experiences, so check the names and the search rank."]
    lines += ["", "Live numbers from Roblox's public web APIs at the time of the call; \"playing now\" changes by the minute.",
              "", "JSON:", json.dumps(res, separators=(",", ":"), ensure_ascii=False)]
    return "\n".join(lines)


def run(action, a=None):
    a = a or {}
    action = (action or "search").lower()
    if action == "search":
        res = search(a.get("query"), a.get("limit"))
        return report(res, "Roblox experiences matching \"%s\", most players first (%d shown of %d matched):"
                      % (res["query"], len(res["games"]), res.get("matched", 0)))
    if action == "details":
        res = details(a.get("universe_ids") or [])
        return report(res, "Roblox experiences by universe id, most players first:")
    raise MarketError("Unknown action '%s'. Use search or details." % action)


def _cmd(a):
    print(run(a.action, {"query": " ".join(a.what), "limit": a.limit, "universe_ids": a.what}))


def register(sub):
    g = sub.add_parser("market", help="look up live Roblox experiences (public data, no login)")
    g.add_argument("action", choices=["search", "details"])
    g.add_argument("what", nargs="+", help="search words, or universe ids for details")
    g.add_argument("--limit", type=int, default=10, help="search: how many to show (20 at most)")
    g.set_defaults(fn=_cmd)
