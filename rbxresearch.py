"""What is working on Roblox right now: the public charts, summarised for finding a game to build.

  rbx.py research snapshot              the summary: genres, openings, rising games, the top of each chart
  rbx.py research snapshot --out f.json also save everything as JSON (the app's New project page reads this)
  rbx.py research chart top-earning     one chart in full

It reads the charts the Roblox website shows a visitor who is not signed in (Top Playing Now, Top Trending,
Up-and-Coming, Top Earning, Top Rated, and the trending list of each genre), then looks up each game's age,
visits and genre. Nothing is sent but a random session id.

What the numbers can and cannot say: "players" is how many are in a game at this moment, not an average; each
chart shows only about its top hundred; a game that is in a chart has already succeeded, so the failures are
invisible; nothing here measures how long players stay or what a game earns, only that it is in Top Earning.
"""
import datetime, json, statistics, time, uuid

from rbxmarket import GAMES, MarketError, _get

EXPLORE = "https://apis.roblox.com/explore-api/v1/get-sorts"
MAIN = ["top-playing-now", "top-trending", "up-and-coming", "top-earning", "top-rated", "top-revisited"]
YOUNG_DAYS, NEW_DAYS = 365, 120


def _charts():
    """Every chart Roblox lists, in its order: {id: {"name", "games": [raw rows]}}."""
    session, token, out = str(uuid.uuid4()), None, {}
    for _ in range(8):
        page = _get(EXPLORE, dict({"sessionId": session}, **({"sortsPageToken": token} if token else {})))
        for s in page.get("sorts", []):
            if s.get("games"):
                out[s["sortId"]] = {"name": s.get("sortDisplayName") or s["sortId"], "games": s["games"]}
        token = page.get("nextSortsPageToken")
        if not token:
            break
    if not out:
        raise MarketError("Roblox's charts came back empty. The endpoint may have changed.")
    return out


def _details(ids):
    """Age, visits and genre for many games. Roblox limits how fast this may be asked, so the requests are
    spaced out and retried; whatever could not be fetched is simply left out (those games show "?" for age)."""
    out = {}
    for i in range(0, len(ids), 50):
        for attempt in range(4):
            try:
                for g in _get(GAMES, {"universeIds": ",".join(map(str, ids[i:i + 50]))}).get("data", []):
                    out[g["id"]] = g
                break
            except MarketError as e:
                if "rate-limiting" not in str(e):
                    break  # this batch cannot be read; carry on with the rest
                if attempt == 3:
                    return out  # still limited after waiting: stop asking
                time.sleep(8 * (attempt + 1))
        time.sleep(1.0)
    return out


def _days_since(stamp, now):
    try:
        then = datetime.datetime.fromisoformat(str(stamp).replace("Z", "+00:00"))
        return max(0, int((now - then).total_seconds() // 86400))
    except ValueError:
        return None


def _game(raw, extra, now):
    up, down = raw.get("totalUpVotes") or 0, raw.get("totalDownVotes") or 0
    return {
        "id": raw["universeId"], "place": raw.get("rootPlaceId"), "name": (raw.get("name") or "").strip(),
        "players": raw.get("playerCount") or 0, "likes": round(100 * up / (up + down)) if up + down else None, "votes": up + down,
        "genre": extra.get("genre_l1") or raw.get("genreL1") or "Unsorted", "subgenre": extra.get("genre_l2") or "",
        "age_days": _days_since(extra.get("created"), now), "updated_days": _days_since(extra.get("updated"), now),
        "visits": extra.get("visits"), "favorites": extra.get("favoritedCount"),
    }


def _median(values):
    values = [v for v in values if v is not None]
    return round(statistics.median(values)) if values else None


def _genres(charts):
    """One row per genre, from the Top Playing Now chart: where the players are and how open it looks."""
    top = charts["top-playing-now"]["games"]
    total = sum(g["players"] for g in top) or 1
    in_chart = lambda chart, genre: sum(1 for g in charts.get(chart, {}).get("games", []) if g["genre"] == genre)
    rows = []
    for genre in sorted({g["genre"] for g in top}):
        games = sorted((g for g in top if g["genre"] == genre), key=lambda g: -g["players"])
        players = sum(g["players"] for g in games)
        young = [g for g in games if g["age_days"] is not None and g["age_days"] <= YOUNG_DAYS]
        rows.append({
            "genre": genre, "games": len(games), "players": players, "share": round(100 * players / total, 1),
            "likes": _median(g["likes"] for g in games), "leader": games[0]["name"], "leader_share": round(100 * games[0]["players"] / (players or 1)),
            "young_games": len(young), "young_share": round(100 * sum(g["players"] for g in young) / (players or 1)),
            "earning": in_chart("top-earning", genre), "rising": in_chart("up-and-coming", genre),
        })
    return sorted(rows, key=lambda r: -r["players"])


def _signals(genres, overall_likes):
    """Plain observations worth a second look. Each one is a reason to dig, not a conclusion."""
    out = []
    for r in genres:
        if r["games"] < 3:
            continue
        g = r["genre"]
        if r["likes"] is not None and overall_likes is not None and r["likes"] <= overall_likes - 4 and r["share"] >= 3:
            out.append("%s holds %s%% of the players in Top Playing Now, but its games have a median like ratio of %d%% against "
                       "%d%% overall: players are there and less satisfied." % (g, r["share"], r["likes"], overall_likes))
        if r["young_share"] >= 40 and r["young_games"] >= 2:
            out.append("%s: %d of its %d charting games are under a year old and hold %d%% of its players: newcomers are getting in."
                       % (g, r["young_games"], r["games"], r["young_share"]))
        if r["leader_share"] >= 60:
            out.append("%s is dominated by one game (%s has %d%% of its players): hard to enter head-on, open to something different."
                       % (g, r["leader"], r["leader_share"]))
        if r["earning"] >= max(3, r["games"]):
            out.append("%s has %d games in Top Earning against %d in Top Playing Now: it earns well for how crowded it is."
                       % (g, r["earning"], r["games"]))
        if r["rising"] >= 8:
            out.append("%s has %d games in Up-and-Coming: a lot of new releases are landing here." % (g, r["rising"]))
    return out


def snapshot():
    """Fetch the charts and work out the summary. Returns one dict (see the keys at the end)."""
    now = datetime.datetime.now(datetime.timezone.utc)
    raw = _charts()
    if "top-playing-now" not in raw:
        raise MarketError("Roblox's charts no longer include Top Playing Now. The endpoint has changed.")
    ids = list(dict.fromkeys(g["universeId"] for c in raw.values() for g in c["games"]))
    extra = _details(ids)
    charts = {cid: {"name": c["name"], "games": [_game(g, extra.get(g["universeId"], {}), now) for g in c["games"] if not g.get("isSponsored")]}
              for cid, c in raw.items()}
    genres = _genres(charts)
    seen, rising = set(), []
    for cid, c in charts.items():
        if cid in ("up-and-coming", "top-trending") or cid.startswith("trending-in-"):
            for g in c["games"]:
                if g["id"] not in seen and g["age_days"] is not None and g["age_days"] <= NEW_DAYS:
                    seen.add(g["id"])
                    rising.append(g)
    rising.sort(key=lambda g: -g["players"])
    overall = _median(g["likes"] for g in charts["top-playing-now"]["games"])
    return {"taken": time.time(), "date": now.strftime("%Y-%m-%d %H:%M UTC"), "games_seen": len(ids), "details_for": len(extra),
            "players_in_top": sum(g["players"] for g in charts["top-playing-now"]["games"]), "median_likes": overall,
            "genres": genres, "signals": _signals(genres, overall), "rising": rising[:40], "charts": charts}


def _count(n):
    if n is None:
        return "?"
    for size, unit in ((1e9, "B"), (1e6, "M"), (1e3, "K")):
        if n >= size:
            return "%.1f%s" % (n / size, unit)
    return str(int(n))


def _age(days):
    if days is None:
        return "?"
    return "%dd" % days if days < 60 else "%dmo" % (days // 30) if days < 730 else "%dy" % (days // 365)


def _line(k, g):
    kind = g["genre"] + (" / " + g["subgenre"] if g["subgenre"] else "")
    return "%2d. %s | %s playing | %s%% likes | %s | %s old | %s visits | id %d" % (
        k, g["name"][:60], _count(g["players"]), "?" if g["likes"] is None else g["likes"], kind, _age(g["age_days"]), _count(g["visits"]), g["id"])


def chart_text(snap, chart, limit=100):
    c = snap["charts"].get(chart)
    if not c:
        raise MarketError("There is no chart called \"%s\". Charts: %s" % (chart, ", ".join(snap["charts"])))
    return "%s (%s), as of %s\n" % (c["name"], chart, snap["date"]) + "\n".join(_line(k, g) for k, g in enumerate(c["games"][:limit], 1))


def digest(snap):
    """The summary an agent reads: small enough to take in at once, with ids for looking closer."""
    out = ["Roblox public charts, %s. %d games seen across %d charts. Top Playing Now holds %s players; median like ratio %s%%."
           % (snap["date"], snap["games_seen"], len(snap["charts"]), _count(snap["players_in_top"]), snap["median_likes"]),
           *(["Roblox limited the lookups: age and visits are missing for %d of the games (shown as ?)." % (snap["games_seen"] - snap["details_for"])]
             if snap["details_for"] < snap["games_seen"] else []),
           "", "GENRES (from Top Playing Now; young = under a year old; earning / rising = how many of the genre are in those charts)",
           "genre | games | players | share | median likes | leader (its share of the genre) | young games (their share) | earning | rising"]
    for r in snap["genres"]:
        out.append("%s | %d | %s | %s%% | %s%% | %s (%d%%) | %d (%d%%) | %d | %d" % (
            r["genre"], r["games"], _count(r["players"]), r["share"], "?" if r["likes"] is None else r["likes"], r["leader"][:40],
            r["leader_share"], r["young_games"], r["young_share"], r["earning"], r["rising"]))
    out += ["", "WORTH A SECOND LOOK (reasons to dig, not conclusions)"] + ["- " + s for s in snap["signals"] or ["nothing stood out by the rules used"]]
    out += ["", "NEW GAMES ALREADY CHARTING (under %d days old, from the trending and up-and-coming lists)" % NEW_DAYS]
    out += [_line(k, g) for k, g in enumerate(snap["rising"][:20], 1)] or ["none"]
    for cid in MAIN:
        if cid in snap["charts"]:
            out += ["", snap["charts"][cid]["name"].upper() + " (top 15 of %d)" % len(snap["charts"][cid]["games"])]
            out += [_line(k, g) for k, g in enumerate(snap["charts"][cid]["games"][:15], 1)]
    others = [c for c in snap["charts"] if c not in MAIN]
    out += ["", "More charts, readable in full with action \"chart\": " + ", ".join(MAIN + others),
            "", "Limits: players are a count at this moment; each chart shows only its top hundred or so; a charting game has already "
            "succeeded, so failures are invisible; nothing here measures retention or revenue."]
    return "\n".join(out)


def run(action, a=None):
    a = a or {}
    snap = snapshot()
    if a.get("out"):
        with open(a["out"], "w", encoding="utf-8") as f:
            json.dump(snap, f)
    if action == "chart":
        return chart_text(snap, str(a.get("chart") or ""))
    if action == "snapshot":
        return digest(snap)
    raise MarketError("action must be \"snapshot\" or \"chart\"")


def _cmd(a):
    print(run(a.action, {"chart": a.chart, "out": a.out}))


def register(sub):
    r = sub.add_parser("research", help="Roblox's public charts summarised: genres, openings, rising games")
    r.add_argument("action", choices=["snapshot", "chart"])
    r.add_argument("chart", nargs="?", help="chart: its id, e.g. top-earning or trending-in-survival")
    r.add_argument("--out", help="also save the whole snapshot as JSON here")
    r.set_defaults(fn=_cmd)
