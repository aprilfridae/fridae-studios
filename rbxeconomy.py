"""Numbers simulator for game progression and drop tables. Pure Python.

  rbx.py economy game.json        simulate the economy described in the file
  rbx.py economy --example        print an example input for each mode
  rbx.py economy --self-check     run the built-in checks against hand-worked cases

PROGRESSION (mode "progression", the default). One player who buys the moment
they can afford the next thing:

  {"currency": "Coins",
   "income": {"per_second": 0, "per_action": 1, "actions_per_minute": 60},
   "start_balance": 0,
   "upgrades": [
     {"name": "Pickaxe", "base_cost": 10, "growth": {"type": "exponential", "rate": 1.15}, "max_level": 25,
      "effect": {"type": "add", "amount": 1, "to": "per_action"}},
     {"name": "Double Coins", "base_cost": 500, "growth": {"type": "polynomial", "power": 2}, "max_level": 5,
      "effect": {"type": "multiply", "factor": 2}, "requires": {"Pickaxe": 5}}],
   "policy": "cheapest",
   "rebirth": {"requirement": {"balance": 100000}, "requirement_growth": 3, "multiplier": 2, "stacking": "multiply",
               "resets": ["balance", "upgrades"], "max": 3},
   "session": {"minutes": 20, "per_day": 2},
   "milestones": [{"name": "First million", "total_earned": 1000000}],
   "max_hours": 100}

  cost of the next level, where level is the number already owned:
    flat         base_cost
    linear       base_cost + step * level
    polynomial   base_cost * (level + 1) ^ power
    exponential  base_cost * rate ^ level
    or give "costs": [10, 25, 60, ...] to list every level's price
  income per second = ((per_second + adds) * multipliers + (per_action + adds) * multipliers * actions_per_minute / 60)
                      * rebirth multiplier
  policy: "cheapest" (lowest price next), "payback" (shortest wait plus time to earn the price back), "in_order"
  rebirth requirement: {"balance": n} (the player rebirths once that costs no more than the next upgrade, or nothing
    is left to buy) or {"all_upgrades": true}. stacking "multiply": multiplier ^ rebirths; "add": 1 + (multiplier - 1) * rebirths.

DROPS (mode "drops"):

  {"mode": "drops",
   "items": [{"name": "Common", "weight": 70}, {"name": "Rare", "weight": 25}, {"name": "Legendary", "weight": 5, "pity": 40}],
   "set": ["Common", "Rare", "Legendary"], "trials": 20000, "seed": 1}

  Give every item a "weight", or every item a "probability" (0 to 1; anything left over is an empty roll).
  pity N: the item is guaranteed on the Nth attempt after N - 1 attempts without it.
"""
import bisect, json, math, random, sys

from rbxpaths import KitError

EXACT = 2.0 ** 53  # past this, whole numbers stop being exact in Luau
PERCENTILES = (("p50", 0.5), ("p90", 0.9), ("p99", 0.99))
# each kind of price growth, with the number it needs and that number's limits
GROWTH = {"flat": {}, "linear": {"step": {}}, "polynomial": {"power": {"low": 0, "high": 20}},
          "exponential": {"rate": {"low": 0.0001, "high": 1000}}}


class EconomyError(KitError):
    pass


def _num(d, key, where, default=None, low=None, high=None, whole=False):
    v = d.get(key, default) if isinstance(d, dict) else default
    if v is None:
        if default is None:
            raise EconomyError("%s needs \"%s\" (a number)." % (where, key))
        return default
    if isinstance(v, bool) or not isinstance(v, (int, float)) or v != v or v in (float("inf"), float("-inf")):
        raise EconomyError("%s: \"%s\" must be a number, not %s." % (where, key, json.dumps(v)))
    if low is not None and v < low:
        raise EconomyError("%s: \"%s\" must be at least %s (got %s)." % (where, key, low, v))
    if high is not None and v > high:
        raise EconomyError("%s: \"%s\" must be at most %s (got %s)." % (where, key, high, v))
    if whole and v != int(v):
        raise EconomyError("%s: \"%s\" must be a whole number (got %s)." % (where, key, v))
    return int(v) if whole else float(v)


def _choice(d, key, where, options, default):
    v = d.get(key, default) if isinstance(d, dict) else default
    if v not in options:
        raise EconomyError("%s: \"%s\" must be one of %s (got %s)." % (where, key, ", ".join(o for o in options if o), json.dumps(v)))
    return v


def money(v):
    if v is None:
        return "-"
    a = abs(v)
    if a >= 1e15:
        return "%.2e" % v
    for cut, unit in ((1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")):
        if a >= cut:
            return "%.2f%s" % (v / cut, unit)
    return ("%.2f" % v).rstrip("0").rstrip(".") if a < 100 else "%.0f" % v


def clock(s):
    if s is None:
        return "-"
    if s < 60:
        return "%.1fs" % s if s < 10 and s != int(s) else "%ds" % round(s)
    if s < 3600:
        return "%dm %02ds" % (s // 60, s % 60)
    if s < 86400:
        return "%dh %02dm" % (s // 3600, s % 3600 // 60)
    return "%dd %02dh" % (s // 86400, s % 86400 // 3600)


def table(head, rows):
    cols = [[str(h)] + [str(r[i]) for r in rows] for i, h in enumerate(head)]
    width = [max(len(c) for c in col) for col in cols]
    out = []
    for n in range(len(rows) + 1):
        out.append("  ".join(cols[i][n].ljust(width[i]) if i == 0 or not _numeric(cols[i][1:]) else cols[i][n].rjust(width[i])
                             for i in range(len(head))).rstrip())
    return "\n".join(out)


def _numeric(cells):
    return bool(cells) and all(c[:1].isdigit() or c[:1] in "-." for c in cells)


# -------------------------------------------------------------- progression
def _upgrades(cfg):
    ups = cfg.get("upgrades")
    if not isinstance(ups, list) or not ups:
        raise EconomyError("Give \"upgrades\": a list with at least one upgrade or item to buy.")
    out, names = [], set()
    for i, u in enumerate(ups):
        if not isinstance(u, dict):
            raise EconomyError("upgrades[%d] must be an object." % i)
        name = str(u.get("name") or "Upgrade %d" % (i + 1))
        where = "upgrade \"%s\"" % name
        if name in names:
            raise EconomyError("Two upgrades are both called \"%s\". Give each its own name." % name)
        names.add(name)
        costs = u.get("costs")
        if costs is not None:
            if not isinstance(costs, list) or not costs or any(isinstance(c, bool) or not isinstance(c, (int, float)) or c < 0 for c in costs):
                raise EconomyError("%s: \"costs\" must be a list of prices, zero or more each." % where)
            max_level, g, base = len(costs), {"type": "list"}, 0.0
        else:
            base = _num(u, "base_cost", where, low=0)
            g = u.get("growth") or {"type": "flat"}
            if isinstance(g, str):
                g = {"type": g}
            if not isinstance(g, dict):
                raise EconomyError("%s: \"growth\" must be an object such as {\"type\": \"exponential\", \"rate\": 1.15}." % where)
            kind = _choice(g, "type", where + " growth", tuple(GROWTH), "flat")
            g = {"type": kind, **{k: _num(g, k, "%s growth (%s)" % (where, kind), **limits) for k, limits in GROWTH[kind].items()}}
            max_level = u.get("max_level", 1)
            if max_level is not None:
                max_level = _num(u, "max_level", where, default=1, low=1, high=100000, whole=True)
        e = u.get("effect") or {"type": "none"}
        if not isinstance(e, dict):
            raise EconomyError("%s: \"effect\" must be an object such as {\"type\": \"add\", \"amount\": 1}." % where)
        kind = _choice(e, "type", where + " effect", ("add", "multiply", "none"), "none")
        if kind == "add":
            e = {"type": kind, "amount": _num(e, "amount", where + " effect (add)"),
                 "to": _choice(e, "to", where + " effect", ("per_second", "per_action", None), None)}
        elif kind == "multiply":
            e = {"type": kind, "factor": _num(e, "factor", where + " effect (multiply)", low=0),
                 "to": _choice(e, "to", where + " effect", ("all", "per_second", "per_action"), "all")}
        else:
            e = {"type": "none"}
        req = u.get("requires") or {}
        if not isinstance(req, dict):
            raise EconomyError("%s: \"requires\" must look like {\"Other upgrade\": 3}." % where)
        req = {other: _num(req, other, where + " requires", low=0) for other in req}
        out.append({"name": name, "base": base, "growth": g, "costs": costs, "max": max_level, "effect": e, "requires": req})
    for u in out:
        for other in u["requires"]:
            if other not in names:
                raise EconomyError("upgrade \"%s\" requires \"%s\", which is not in the list." % (u["name"], other))
    return out


def cost_of(u, level):
    g = u["growth"]
    try:
        if g["type"] == "list":
            return float(u["costs"][level])
        if g["type"] == "linear":
            return max(0.0, u["base"] + g["step"] * level)
        if g["type"] == "polynomial":
            return u["base"] * (level + 1) ** g["power"]
        if g["type"] == "exponential":
            return u["base"] * g["rate"] ** level
    except OverflowError:
        return float("inf")
    return u["base"]


def _rebirth(rb):
    """The checked rebirth rule, or None when the game has none."""
    if rb is None:
        return None
    if not isinstance(rb, dict):
        raise EconomyError("\"rebirth\" must be an object.")
    rq = rb.get("requirement") or {}
    if not isinstance(rq, dict) or not (("balance" in rq) != bool(rq.get("all_upgrades"))):
        raise EconomyError("rebirth: \"requirement\" must be {\"balance\": n} or {\"all_upgrades\": true}.")
    resets = rb.get("resets", ["balance", "upgrades"])
    if not isinstance(resets, list) or any(r not in ("balance", "upgrades") for r in resets):
        raise EconomyError("rebirth: \"resets\" is a list that may hold \"balance\" and \"upgrades\".")
    return {"balance": _num(rq, "balance", "rebirth requirement", low=0) if "balance" in rq else None,
            "growth": _num(rb, "requirement_growth", "rebirth", default=1.0, low=0.0001, high=1e6),
            "mult": _num(rb, "multiplier", "rebirth", default=2.0, low=0),
            "stacking": _choice(rb, "stacking", "rebirth", ("multiply", "add"), "multiply"),
            "resets": resets, "max": _num(rb, "max", "rebirth", default=3, low=1, high=1000, whole=True),
            "keep": [str(k) for k in rb.get("keep") or []]}


def _milestones(cfg):
    goals = []
    for i, m in enumerate(cfg.get("milestones") or []):
        if not isinstance(m, dict) or not (("total_earned" in m) != ("income_per_second" in m)):
            raise EconomyError("milestones[%d] needs a \"name\" and either \"total_earned\" or \"income_per_second\"." % i)
        kind = "total_earned" if "total_earned" in m else "income_per_second"
        goals.append({"name": str(m.get("name") or "%s %s" % (kind, m[kind])), "kind": kind,
                      "value": _num(m, kind, "milestone", low=0), "time": None})
    return goals


def simulate(cfg):
    """Run the progression. Returns a dict of results (see report())."""
    inc = cfg.get("income")
    if not isinstance(inc, dict):
        raise EconomyError("Give \"income\": {\"per_second\": n} or {\"per_action\": n, \"actions_per_minute\": n}.")
    base_ps = _num(inc, "per_second", "income", default=0.0, low=0)
    base_pa = _num(inc, "per_action", "income", default=0.0, low=0)
    apm = _num(inc, "actions_per_minute", "income", default=60.0, low=0, high=6000)
    add_to = "per_action" if "per_action" in inc else "per_second"
    ups = _upgrades(cfg)
    policy = _choice(cfg, "policy", "the economy", ("cheapest", "payback", "in_order"), "cheapest")
    horizon = _num(cfg, "max_hours", "the economy", default=100.0, low=0.01, high=100000) * 3600
    bal = _num(cfg, "start_balance", "the economy", default=0.0, low=0)
    rb = _rebirth(cfg.get("rebirth"))
    goals = _milestones(cfg)

    levels = {u["name"]: 0 for u in ups}
    rebirths = 0

    def income():
        a_ps = a_pa = 0.0
        m_ps = m_pa = 1.0
        for u in ups:
            lv, e = levels[u["name"]], u["effect"]
            if not lv:
                continue
            if e["type"] == "add":
                if (e["to"] or add_to) == "per_second":
                    a_ps += e["amount"] * lv
                else:
                    a_pa += e["amount"] * lv
            elif e["type"] == "multiply":
                try:
                    f = e["factor"] ** lv
                except OverflowError:
                    f = float("inf")
                if e["to"] in ("all", "per_second"):
                    m_ps *= f
                if e["to"] in ("all", "per_action"):
                    m_pa *= f
        r = 1.0
        if rb and rebirths:
            r = rb["mult"] ** rebirths if rb["stacking"] == "multiply" else 1 + (rb["mult"] - 1) * rebirths
        return max(0.0, ((base_ps + a_ps) * m_ps + (base_pa + a_pa) * m_pa * apm / 60.0) * r)

    def open_upgrades():
        out = []
        for u in ups:
            lv = levels[u["name"]]
            if u["max"] is not None and lv >= u["max"]:
                continue
            if any(levels[o] < need for o, need in u["requires"].items()):
                continue
            out.append((u, cost_of(u, lv)))
        return out

    def pick(avail, rate):
        if policy == "in_order":
            return avail[0]
        if policy == "cheapest":
            return min(avail, key=lambda x: x[1])
        best, score = None, None
        for u, c in avail:  # payback: wait until affordable, plus how long the purchase takes to pay for itself
            levels[u["name"]] += 1
            gain = income() - rate
            levels[u["name"]] -= 1
            wait = max(0.0, c - bal) / rate if rate > 0 else float("inf")
            s = wait + (c / gain if gain > 0 else float("inf"))
            if score is None or s < score:
                best, score = (u, c), s
        if score == float("inf"):
            return min(avail, key=lambda x: x[1])
        return best

    def earn(seconds):
        """Play for this long at the current rate, noting the milestones passed on the way."""
        nonlocal t, earned, bal
        for g in goals:
            if g["time"] is None and g["kind"] == "total_earned" and earned + rate * seconds >= g["value"]:
                g["time"] = t + (max(0.0, g["value"] - earned) / rate if rate > 0 else 0.0)
        t += seconds
        earned += rate * seconds
        bal += rate * seconds

    t = earned = 0.0
    start_income = income()
    events, stop, biggest = [], None, max(bal, start_income)
    while True:
        if len(events) >= 100000:
            stop = "stopped after 100000 purchases"
            break
        rate = income()
        avail = open_upgrades()
        target = pick(avail, rate) if avail else None
        is_rebirth = False
        if rb and rebirths < rb["max"]:
            if rb["balance"] is None:
                if all(u["max"] is not None and levels[u["name"]] >= u["max"] for u in ups):
                    target, is_rebirth = (None, 0.0), True
            else:
                need = rb["balance"] * rb["growth"] ** rebirths
                if target is None or need <= target[1]:
                    target, is_rebirth = (None, need), True
        if target is None:
            locked = [u["name"] for u in ups if u["max"] is None or levels[u["name"]] < u["max"]]
            stop = "finished" if not locked else "stuck: %s can never unlock" % ", ".join(locked[:4])
            break
        cost = target[1]
        wait = 0.0
        if cost > bal:
            if rate <= 0 or cost == float("inf"):
                stop = "stalled: no income to pay for %s (%s)" % ("the rebirth" if is_rebirth else target[0]["name"], money(cost))
                break
            wait = (cost - bal) / rate
        if t + wait > horizon:
            stop = "not finished: the next purchase (%s, %s) is %s away" % (
                "rebirth" if is_rebirth else target[0]["name"], money(cost), clock(wait))
            earn(horizon - t)  # play out the remaining time so the totals are right
            t = horizon
            break
        earn(wait)
        biggest = max(biggest, bal, cost)
        if is_rebirth:
            rebirths += 1
            if "balance" in rb["resets"]:
                bal = 0.0
            elif rb["balance"] is not None:
                bal = max(0.0, bal - cost)
            if "upgrades" in rb["resets"]:
                for u in ups:
                    if u["name"] not in rb["keep"]:
                        levels[u["name"]] = 0
            what, lv = "REBIRTH", rebirths
        else:
            bal = max(0.0, bal - cost)
            levels[target[0]["name"]] += 1
            what, lv = target[0]["name"], levels[target[0]["name"]]
        after = income()
        biggest = max(biggest, after)
        events.append({"n": len(events) + 1, "time": t, "wait": wait, "what": what, "level": lv, "cost": cost,
                       "income": after, "earned": earned, "rebirth": is_rebirth})
        for g in goals:
            if g["time"] is None and g["kind"] == "income_per_second" and after >= g["value"]:
                g["time"] = t

    longest = max(events, key=lambda e: e["wait"]) if events else None
    res = {"currency": str(cfg.get("currency") or "coins"), "policy": policy, "start_income": start_income,
           "end_income": income(), "purchases": len(events), "rebirths": rebirths, "ended": stop,
           "finished": stop == "finished", "time": t, "total_earned": earned, "events": events,
           "milestones": [{"name": g["name"], "time": g["time"]} for g in goals],
           "first_purchase": events[0]["time"] if events else None,
           "longest_wait": {"seconds": longest["wait"], "before": "%s level %d" % (longest["what"], longest["level"]),
                            "purchase": longest["n"], "at": longest["time"]} if longest else None,
           "walls": _walls(events)}
    res["flags"] = _flags(res, biggest)
    if cfg.get("session") is not None:
        res["session"] = _session(cfg["session"], res)
    return res


def _walls(events):
    """Purchases whose wait is at least a minute and over 3x the longest wait before them."""
    walls, top = [], 0.0
    for e in events:
        if e["n"] > 1 and e["wait"] >= 60 and e["wait"] > 3 * max(top, 5.0):
            walls.append({"purchase": e["n"], "before": "%s level %d" % (e["what"], e["level"]), "wait": e["wait"],
                          "longest_before": top, "jump": e["wait"] / max(top, 5.0)})
        top = max(top, e["wait"])
    return walls


def _flags(res, biggest):
    """What a designer should look at, in plain sentences. biggest: the largest value the game ever shows."""
    events, walls, stop, t = res["events"], res["walls"], res["ended"], res["time"]
    flags = []
    if not events:
        flags.append("Nothing is ever bought: " + (stop or ""))
    elif res["first_purchase"] > 60:
        flags.append("First purchase takes %s. New players should be able to buy something within about a minute."
                     % clock(res["first_purchase"]))
    for w in walls[:5]:
        flags.append("Wall before purchase %d (%s): a wait of %s, %.1fx the longest wait before it (%s)."
                     % (w["purchase"], w["before"], clock(w["wait"]), w["jump"], clock(w["longest_before"])))
    if len(walls) > 5:
        flags.append("... and %d more walls." % (len(walls) - 5))
    if biggest >= EXACT:
        flags.append("Runaway inflation: values reach %s. Past 9.0e15 whole numbers stop being exact, and an IntValue cannot hold "
                     "more than 9.2e18: plan for number abbreviations and number storage, or flatten the growth." % money(biggest))
    waits = [e["wait"] for e in events if not e["rebirth"]]
    if len(waits) >= 12:
        third = len(waits) // 3
        mid, late = sorted(waits[third:2 * third]), sorted(waits[2 * third:])
        m_mid, m_late = mid[len(mid) // 2], late[len(late) // 2]
        if m_late < 2 and m_mid > 10 * max(m_late, 0.01):
            flags.append("Runaway inflation: income outgrows prices. The typical wait falls from %s in the middle of the game to "
                         "%s at the end, so late purchases are instant." % (clock(m_mid), clock(m_late)))
    if res["finished"] and t < 3600:
        flags.append("All content is bought after %s of play: under an hour." % clock(t))
    if stop and stop != "finished":
        flags.append("The run ended early: %s (after %s of play)." % (stop, clock(t)))
    return flags


def _session(ses, res):
    """The run measured in play sessions and days."""
    if not isinstance(ses, dict):
        raise EconomyError("\"session\" must be an object such as {\"minutes\": 20, \"per_day\": 2}.")
    mins = _num(ses, "minutes", "session", low=0.1, high=1440)
    per_day = _num(ses, "per_day", "session", default=1.0, low=0.01, high=100)

    def day(sec):
        return None if sec is None else max(1, math.ceil(math.ceil(max(sec, 1e-9) / (mins * 60)) / per_day))
    return {"minutes": mins, "per_day": per_day,
            "purchases_in_first_session": sum(1 for e in res["events"] if e["time"] <= mins * 60),
            "first_purchase_day": day(res["first_purchase"]),
            "end_day": day(res["time"]), "play_minutes_per_day": mins * per_day}


def _sample(events, cap=40):
    if len(events) <= cap:
        return events, False
    keep = set(range(10)) | {len(events) - 1} | {i for i, e in enumerate(events) if e["rebirth"]}
    step = (len(events) - 10) / float(cap - 10)
    keep |= {10 + int(i * step) for i in range(cap - 10)}
    return [events[i] for i in sorted(keep) if i < len(events)], True


def report(res):
    """Text tables and a JSON block for a simulate() result."""
    ev, sampled = _sample(res["events"])
    lines = ["PROGRESSION (%s, policy: %s). Income only while playing; the player buys as soon as they can." % (res["currency"], res["policy"]),
             "", "Timeline%s:" % (" (%d of %d purchases shown)" % (len(ev), len(res["events"])) if sampled else "")]
    lines.append(table(["#", "time", "wait", "bought", "lvl", "cost", "income/s after"],
                       [[e["n"], clock(e["time"]), clock(e["wait"]), e["what"], e["level"], money(e["cost"]), money(e["income"])] for e in ev])
                 if ev else "  (no purchases)")
    marks = [m for m in (60, 300, 900, 1800, 3600, 7200, 14400, 28800, 86400, 172800, 360000) if m <= res["time"]]
    if marks and res["events"]:
        rows, i, rate, earned, last = [], 0, res["start_income"], 0.0, 0.0
        evs = res["events"]
        for m in marks:
            while i < len(evs) and evs[i]["time"] <= m:
                rate, earned, last = evs[i]["income"], evs[i]["earned"], evs[i]["time"]
                i += 1
            rows.append([clock(m), money(rate), money(earned + rate * (m - last)), i])
        lines += ["", "Income over time:", table(["after", "income/s", "total earned", "purchases"], rows)]
    lw = res["longest_wait"]
    lines += ["", "Summary:",
              "  income per second: %s at the start, %s at the end" % (money(res["start_income"]), money(res["end_income"])),
              "  time to first purchase: %s" % clock(res["first_purchase"]),
              "  time to finish all content: %s" % (clock(res["time"]) if res["finished"] else "not reached (%s)" % res["ended"]),
              "  longest wait between purchases: %s" % ("%s, before purchase %d (%s) at %s" % (
                  clock(lw["seconds"]), lw["purchase"], lw["before"], clock(lw["at"])) if lw else "-"),
              "  purchases: %d, rebirths: %d, total earned: %s" % (res["purchases"], res["rebirths"], money(res["total_earned"]))]
    for m in res["milestones"]:
        lines.append("  milestone \"%s\": %s" % (m["name"], clock(m["time"]) if m["time"] is not None else "not reached"))
    s = res.get("session")
    if s:
        lines.append("  at %g minutes a session, %g a day: %d purchase(s) in the first session; first purchase on day %s; %s on day %s"
                     % (s["minutes"], s["per_day"], s["purchases_in_first_session"], s["first_purchase_day"] or "-",
                        "everything bought" if res["finished"] else "the run ends", s["end_day"]))
    lines += ["", "Flags:"] + (["  - " + f for f in res["flags"]] or ["  none"])
    slim = {k: v for k, v in res.items() if k != "events"}
    slim["timeline"] = [{"n": e["n"], "time": round(e["time"], 2), "wait": round(e["wait"], 2), "bought": e["what"], "level": e["level"],
                         "cost": float("%.6g" % e["cost"]), "income_after": float("%.6g" % e["income"])} for e in ev]
    slim["timeline_sampled"] = sampled
    lines += ["", "JSON:", json.dumps(slim, separators=(",", ":"))]
    return "\n".join(lines)


# -------------------------------------------------------------------- drops
def _pct(sorted_values, p):
    return sorted_values[min(len(sorted_values) - 1, max(0, math.ceil(p * len(sorted_values)) - 1))]


def _geometric(rng, q):
    """Attempts until the first success when each succeeds with chance q."""
    if q >= 1:
        return 1
    return max(1, math.ceil(math.log(1.0 - rng.random()) / math.log(1.0 - q)))


def drops(cfg):
    items = cfg.get("items")
    if not isinstance(items, list) or not items:
        raise EconomyError("Give \"items\": a list such as [{\"name\": \"Rare\", \"weight\": 5}].")
    names, raw, pity = [], [], {}
    use_w = all(isinstance(i, dict) and "weight" in i for i in items)
    use_p = all(isinstance(i, dict) and "probability" in i for i in items)
    if not (use_w or use_p) or any(isinstance(i, dict) and "weight" in i and "probability" in i for i in items):
        raise EconomyError("Give every item a \"weight\", or every item a \"probability\" (not a mix).")
    for n, it in enumerate(items):
        name = str(it.get("name") or "Item %d" % (n + 1))
        if name in names:
            raise EconomyError("Two items are both called \"%s\"." % name)
        names.append(name)
        raw.append(_num(it, "weight" if use_w else "probability", "item \"%s\"" % name, low=0, high=None if use_w else 1))
        if it.get("pity") is not None:
            pity[name] = _num(it, "pity", "item \"%s\"" % name, low=1, high=10000000, whole=True)
    total = sum(raw)
    if total <= 0:
        raise EconomyError("The %s add up to zero: nothing can drop." % ("weights" if use_w else "probabilities"))
    if use_p and total > 1 + 1e-9:
        raise EconomyError("The probabilities add up to %.6g, which is more than 1." % total)
    prob = {n: (r / total if use_w else r) for n, r in zip(names, raw)}
    nothing = 0.0 if use_w else max(0.0, 1.0 - total)
    wanted = cfg.get("set") or names
    if not isinstance(wanted, list) or any(w not in prob for w in wanted):
        raise EconomyError("\"set\" must list item names from \"items\".")
    never = [n for n in wanted if prob[n] <= 0 and n not in pity]
    if never:
        raise EconomyError("%s can never drop (chance 0 and no pity), so the set can never be completed." % ", ".join(never))
    trials = _num(cfg, "trials", "drops", default=20000, low=100, high=200000, whole=True)
    rng = random.Random(cfg.get("seed", 12345))
    out = {"items": [], "empty_roll_chance": nothing, "set": wanted}

    for n in names:
        p, cap = prob[n], pity.get(n)
        if cap:
            expected = (1 - (1 - p) ** cap) / p if p > 0 else float(cap)
        else:
            expected = 1 / p if p > 0 else None
        out["items"].append({"name": n, "chance": p, "one_in": 1 / p if p > 0 else None, "pity": cap, "expected_attempts": expected})

    full = []  # attempts to complete the set, one entry per trial
    if not pity:
        # exact per-item percentiles; the set is simulated by jumping from one new item to the next
        for row in out["items"]:
            p = row["chance"]
            for label, q in PERCENTILES:
                row[label] = None if p <= 0 else (1 if p >= 1 else max(1, math.ceil(math.log(1 - q) / math.log(1 - p) - 1e-12)))
            row["percentiles"] = "exact"
        for _ in range(trials):
            left, t = list(wanted), 0
            while left:
                weights = [prob[n] for n in left]
                q = sum(weights)
                t += _geometric(rng, q)
                x, acc = rng.random() * q, 0.0
                for k, w in enumerate(weights):
                    acc += w
                    if x < acc or k == len(left) - 1:
                        left.pop(k)
                        break
            full.append(t)
        out["trials"] = trials
        if len(wanted) <= 16:
            exp = 0.0
            for mask in range(1, 1 << len(wanted)):
                q = sum(prob[wanted[i]] for i in range(len(wanted)) if mask >> i & 1)
                exp += (1 if bin(mask).count("1") % 2 else -1) / q
            out["set_expected_exact"] = exp
    else:
        # every attempt is played out, because pity depends on the history
        order = sorted(pity, key=lambda n: prob[n])
        cum, acc = [], 0.0
        for n in names:
            acc += prob[n]
            cum.append(acc)
        can_drop = sum(1 for n in names if prob[n] > 0 or n in pity)
        firsts = {n: [] for n in names}
        budget, done = 3000000, 0
        while done < trials and (budget > 0 or done < 200):
            since = dict.fromkeys(pity, 0)
            seen, t, missing, got_set = {}, 0, can_drop, None
            want_left = set(wanted)
            while missing and t < 2000000:
                t += 1
                k = bisect.bisect_right(cum, rng.random())
                got = names[k] if k < len(names) else None
                for n in order:
                    if since[n] + 1 >= pity[n] and got != n:
                        got = n
                        break
                for n in since:
                    since[n] = 0 if n == got else since[n] + 1
                if got is not None and got not in seen:
                    seen[got] = t
                    missing -= 1
                    want_left.discard(got)
                    if not want_left and got_set is None:
                        got_set = t
            budget -= t
            done += 1
            for n, first in seen.items():
                firsts[n].append(first)
            if got_set is not None:
                full.append(got_set)
        out["trials"] = done
        if done < trials:
            out["note"] = "Fewer trials than asked (%d of %d): each one plays out every attempt, and this table needs many." % (done, trials)
        for row in out["items"]:
            v = sorted(firsts[row["name"]])
            row["percentiles"] = "simulated"
            for label, q in PERCENTILES:
                row[label] = _pct(v, q) if v else None
            row["simulated_mean"] = sum(v) / len(v) if v else None
    full.sort()
    out["set_attempts"] = dict(mean=sum(full) / len(full), **{label: _pct(full, q) for label, q in PERCENTILES}) if full else None
    return out


def report_drops(res):
    rows = []
    for r in res["items"]:
        rows.append([r["name"], "%.4g%%" % (r["chance"] * 100), "1 in %.4g" % r["one_in"] if r["one_in"] else "never by chance",
                     r["pity"] or "-", "%.4g" % r["expected_attempts"] if r["expected_attempts"] else "-",
                     r["p50"] or "-", r["p90"] or "-", r["p99"] or "-"])
    some_pity = any(r["pity"] for r in res["items"])
    lines = ["DROP TABLE (chances are exact; attempts to get each item at least once)", "",
             table(["item", "chance", "odds", "pity", "expected", "p50", "p90", "p99"], rows)]
    if res["empty_roll_chance"] > 1e-12:
        lines.append("An attempt gives nothing %.4g%% of the time." % (res["empty_roll_chance"] * 100))
    lines.append("Percentiles: %s. p90 = 9 players in 10 have it by then." % (
        "simulated over %d trials, because pity depends on history; \"expected\" is each item's own formula and ignores other "
        "items' pity" % res["trials"] if some_pity else "exact"))
    s = res["set_attempts"]
    if s:
        lines += ["", "Full set (%s):" % ", ".join(res["set"]),
                  "  expected attempts: %s" % ("%.4g (exact), %.4g (simulated)" % (res["set_expected_exact"], s["mean"])
                                             if res.get("set_expected_exact") else "%.4g (simulated)" % s["mean"]),
                  "  simulated over %d trials: p50 %d, p90 %d, p99 %d" % (res["trials"], s["p50"], s["p90"], s["p99"])]
    if res.get("note"):
        lines.append(res["note"])
    lines += ["", "JSON:", json.dumps(res, separators=(",", ":"))]
    return "\n".join(lines)


EXAMPLES = {
    "progression": {"currency": "Coins", "income": {"per_action": 1, "actions_per_minute": 60}, "upgrades": [
        {"name": "Pickaxe", "base_cost": 10, "growth": {"type": "exponential", "rate": 1.15}, "max_level": 25,
         "effect": {"type": "add", "amount": 1}},
        {"name": "Double Coins", "base_cost": 500, "growth": {"type": "polynomial", "power": 2}, "max_level": 5,
         "effect": {"type": "multiply", "factor": 2}, "requires": {"Pickaxe": 5}}],
        "policy": "cheapest", "rebirth": {"requirement": {"balance": 100000}, "requirement_growth": 3, "multiplier": 2, "max": 3},
        "session": {"minutes": 20, "per_day": 2}, "milestones": [{"name": "First million", "total_earned": 1000000}]},
    "drops": {"mode": "drops", "items": [{"name": "Common", "weight": 70}, {"name": "Rare", "weight": 25},
                                         {"name": "Legendary", "weight": 5, "pity": 40}], "trials": 20000},
}


def run(cfg):
    """Text for one input object, either mode."""
    if isinstance(cfg, str):
        try:
            cfg = json.loads(cfg)
        except ValueError as e:
            raise EconomyError("The input is not valid JSON: %s" % e)
    if not isinstance(cfg, dict):
        raise EconomyError("The input must be one JSON object. Call with example true to see the shape.")
    mode = cfg.get("mode") or ("drops" if "items" in cfg and "upgrades" not in cfg else "progression")
    if mode == "drops":
        return report_drops(drops(cfg))
    if mode != "progression":
        raise EconomyError("\"mode\" must be \"progression\" or \"drops\" (got %s)." % json.dumps(mode))
    return report(simulate(cfg))


# -------------------------------------------------------------- self-checks
def self_check():
    """Compare against cases worked out by hand. Returns [(name, passed, detail)]."""
    out = []

    def check(name, got, want, tol=1e-9):
        ok = got == want if not isinstance(want, float) else (got is not None and abs(got - want) <= tol * max(1.0, abs(want)))
        out.append((name, ok, "got %s, expected %s" % (got, want)))

    def fails(name, cfg, part):
        try:
            run(cfg)
            out.append((name, False, "no error raised"))
        except EconomyError as e:
            out.append((name, part in str(e), str(e)))

    # 1/s; costs 10, 20, 30; each adds 1/s. Waits 10s, 20/2 = 10s, 30/3 = 10s.
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "A", "base_cost": 10, "growth": {"type": "linear", "step": 10},
                                                              "max_level": 3, "effect": {"type": "add", "amount": 1}}]})
    check("linear: purchase times", [e["time"] for e in r["events"]], [10.0, 20.0, 30.0])
    check("linear: income after", [e["income"] for e in r["events"]], [2.0, 3.0, 4.0])
    check("linear: finish time", r["time"], 30.0)
    check("linear: under-an-hour flag", any("under an hour" in f for f in r["flags"]), True)
    check("linear: total earned", r["total_earned"], 60.0)
    # 10/s; costs 100, 200, 400; each doubles income. Waits 10s, 10s, 10s.
    r = simulate({"income": {"per_second": 10}, "upgrades": [{"name": "M", "base_cost": 100, "growth": {"type": "exponential", "rate": 2},
                                                               "max_level": 3, "effect": {"type": "multiply", "factor": 2}}]})
    check("exponential: purchase times", [e["time"] for e in r["events"]], [10.0, 20.0, 30.0])
    check("exponential: final income", r["end_income"], 80.0)
    # polynomial power 2 from 5: 5, 20, 45. 2 per action, 30 actions a minute = 1/s; no effect. Times 5, 25, 70.
    r = simulate({"income": {"per_action": 2, "actions_per_minute": 30}, "upgrades": [
        {"name": "P", "base_cost": 5, "growth": {"type": "polynomial", "power": 2}, "max_level": 3}]})
    check("polynomial + per action: purchase times", [e["time"] for e in r["events"]], [5.0, 25.0, 70.0])
    # first purchase after 120s is flagged; second costs 10000 at 2/s = 5000s, a wall (more than 3x 120s)
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "S", "costs": [120, 10000], "effect": {"type": "add", "amount": 1}}]})
    check("slow start flag", any(f.startswith("First purchase takes 2m 00s") for f in r["flags"]), True)
    check("wall found at purchase 2", [w["purchase"] for w in r["walls"]], [2])
    check("wall wait", r["walls"][0]["wait"], 5000.0)
    check("longest wait", r["longest_wait"]["seconds"], 5000.0)
    # payback: A costs 10 for +1/s (pays back in 10s), B costs 5 for +0.1/s (50s). Cheapest buys B first, payback buys A first.
    two = [{"name": "A", "base_cost": 10, "effect": {"type": "add", "amount": 1}}, {"name": "B", "base_cost": 5, "effect": {"type": "add", "amount": 0.1}}]
    check("policy cheapest order", [e["what"] for e in simulate({"income": {"per_second": 1}, "upgrades": two})["events"]], ["B", "A"])
    check("policy payback order", [e["what"] for e in simulate({"income": {"per_second": 1}, "upgrades": two, "policy": "payback"})["events"]], ["A", "B"])
    # rebirth: 1/s, one upgrade costing 10 (+1/s), rebirth when all bought, x2, twice, resets everything.
    # run 1: buy at 10s, rebirth at 10s. run 2 at 2/s: buy at 15s (income 4/s), rebirth. run 3 at 4/s: buy at 17.5s. Done.
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 10, "effect": {"type": "add", "amount": 1}}],
                  "rebirth": {"requirement": {"all_upgrades": True}, "multiplier": 2, "max": 2}})
    check("rebirth: event order", [e["what"] for e in r["events"]], ["U", "REBIRTH", "U", "REBIRTH", "U"])
    check("rebirth: times", [e["time"] for e in r["events"]], [10.0, 10.0, 15.0, 15.0, 17.5])
    check("rebirth: final income", r["end_income"], 8.0)
    # balance rebirth: 10/s, upgrade costs 1000, rebirth at 100 (cheaper, so taken first), once, x3, resets balance.
    # rebirth at 10s (30/s), then 1000 / 30 = 33.33s more.
    r = simulate({"income": {"per_second": 10}, "upgrades": [{"name": "U", "base_cost": 1000}],
                  "rebirth": {"requirement": {"balance": 100}, "multiplier": 3, "max": 1}})
    check("balance rebirth: times", [round(e["time"], 4) for e in r["events"]], [10.0, round(10 + 1000 / 30.0, 4)])
    # milestone and sessions: 1/s, upgrade at 3000s; earned 1000 at 1000s; 20-minute sessions, 2 a day -> session 3, day 2
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 3000}], "session": {"minutes": 20, "per_day": 2},
                  "milestones": [{"name": "1k", "total_earned": 1000}]})
    check("milestone time", r["milestones"][0]["time"], 1000.0)
    check("session: day of first purchase", r["session"]["first_purchase_day"], 2)
    # inflation: x10 per level for 20 levels at a flat price passes 2^53
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "X", "base_cost": 1, "max_level": 20, "effect": {"type": "multiply", "factor": 10}}]})
    check("inflation flag", any(f.startswith("Runaway inflation: values reach") for f in r["flags"]), True)
    # no income, cannot afford
    r = simulate({"income": {"per_second": 0}, "upgrades": [{"name": "U", "base_cost": 5}]})
    check("stall is reported", r["ended"].startswith("stalled"), True)
    # horizon
    r = simulate({"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 1e9}], "max_hours": 1})
    check("not finished within max_hours", (r["finished"], r["time"], r["total_earned"]), (False, 3600.0, 3600.0))
    fails("validation: no upgrades", {"income": {"per_second": 1}}, "upgrades")
    fails("validation: bad growth", {"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 1, "growth": {"type": "cubic"}}]}, "must be one of")
    fails("validation: missing rate", {"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 1, "growth": {"type": "exponential"}}]}, "rate")
    fails("validation: negative cost", {"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": -1}]}, "at least 0")
    fails("validation: text for a number", {"income": {"per_second": "fast"}, "upgrades": [{"name": "U", "base_cost": 1}]}, "must be a number")
    fails("validation: unknown requirement", {"income": {"per_second": 1}, "upgrades": [{"name": "U", "base_cost": 1, "requires": {"Z": 1}}]}, "not in the list")

    # drops. Two items at 50%: each expected 2 attempts, the pair 3 (1 + 2).
    d = drops({"items": [{"name": "A", "weight": 1}, {"name": "B", "weight": 1}], "trials": 40000, "seed": 7})
    check("drops: chance", d["items"][0]["chance"], 0.5)
    check("drops: expected attempts", d["items"][0]["expected_attempts"], 2.0)
    check("drops: exact p50 / p90 / p99 at 50%", (d["items"][0]["p50"], d["items"][0]["p90"], d["items"][0]["p99"]), (1, 4, 7))
    check("drops: pair expected (exact)", d["set_expected_exact"], 3.0)
    check("drops: pair expected (simulated)", d["set_attempts"]["mean"], 3.0, tol=0.02)
    # three equal items: 3 * (1 + 1/2 + 1/3) = 5.5
    d = drops({"items": [{"name": n, "weight": 1} for n in "ABC"], "trials": 40000, "seed": 7})
    check("drops: three-item set (exact)", d["set_expected_exact"], 5.5)
    check("drops: three-item set (simulated)", d["set_attempts"]["mean"], 5.5, tol=0.02)
    # 10%: expected 10; p50 = 7 (0.9^7 = 0.478), p90 = 22 (0.9^22 = 0.098), p99 = 44 (0.9^44 = 0.0097)
    d = drops({"items": [{"name": "R", "probability": 0.1}], "trials": 100})
    check("drops: 10% percentiles", (d["items"][0]["p50"], d["items"][0]["p90"], d["items"][0]["p99"]), (7, 22, 44))
    check("drops: empty roll chance", d["empty_roll_chance"], 0.9)
    # 10% with pity 5: expected (1 - 0.9^5) / 0.1 = 4.0951; never more than 5 attempts
    d = drops({"items": [{"name": "R", "probability": 0.1, "pity": 5}], "trials": 40000, "seed": 3})
    check("drops: pity expected (formula)", d["items"][0]["expected_attempts"], 4.0951)
    check("drops: pity expected (simulated)", d["items"][0]["simulated_mean"], 4.0951, tol=0.02)
    check("drops: pity caps p99", d["items"][0]["p99"], 5)
    fails("drops validation: mixed", {"mode": "drops", "items": [{"name": "A", "weight": 1}, {"name": "B", "probability": 0.5}]}, "not a mix")
    fails("drops validation: sum over 1", {"mode": "drops", "items": [{"name": "A", "probability": 0.7}, {"name": "B", "probability": 0.6}]}, "more than 1")
    fails("drops validation: impossible set", {"mode": "drops", "items": [{"name": "A", "weight": 1}, {"name": "B", "weight": 0}]}, "never drop")
    return out


def _cmd(a):
    if a.self_check:
        rows = self_check()
        for name, ok, detail in rows:
            print("%s  %s%s" % ("ok  " if ok else "FAIL", name, "" if ok else "  (" + detail + ")"))
        bad = sum(1 for r in rows if not r[1])
        print("%d checks, %d failed" % (len(rows), bad))
        sys.exit(1 if bad else 0)
    if a.example:
        print(json.dumps(EXAMPLES, indent=1))
        return
    if not a.file:
        sys.exit("give a JSON file (or - for stdin), --example or --self-check")
    try:
        text = sys.stdin.read() if a.file == "-" else open(a.file, encoding="utf-8-sig").read()
    except OSError as e:
        sys.exit("Could not read %s: %s" % (a.file, e))
    print(run(text))


def register(sub):
    g = sub.add_parser("economy", help="simulate a game's progression numbers or a drop table from a JSON file")
    g.add_argument("file", nargs="?", help="JSON file describing the economy or drop table, or - for stdin")
    g.add_argument("--example", action="store_true", help="print an example input for each mode")
    g.add_argument("--self-check", dest="self_check", action="store_true", help="run the built-in checks")
    g.set_defaults(fn=_cmd)
