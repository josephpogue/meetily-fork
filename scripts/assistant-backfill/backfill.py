#!/usr/bin/env python3
"""Rebuild one meeting's assistant history from Claude Code session transcripts.

One-off preview tool for meetily-fork. The assistant's output for the
"PNC - Introductory Call with Jim" meeting exists only in the transcripts of the
assistant's claude CLI sessions. This script parses those replies the way the
Rust code does (assistant/card.rs, assistant/ask_list.rs, assistant/lanes.rs)
and writes them into the meeting_assistant_items table and <folder>/assistant.json.

Default run is a dry run. Run it with `python3 -I backfill.py`. Python stdlib only.
"""

import argparse
import glob
import json
import os
import re
import sqlite3
import sys
import tempfile
import uuid
from datetime import datetime, timezone

DEFAULT_SESSIONS = os.path.expanduser(
    "~/.claude/projects/"
    "-private-var-folders-fy-4w2l5rnd3rd7ysqhfn36hj-w0000gp-T-meetily-assistant-"
    "d27bbcab-3dda-4422-a6e1-939a3909d380"
)
DEFAULT_MEETING = "meeting-10c08825-4e2e-498d-b1ce-252b8de0f071"
DEFAULT_FOLDER = os.path.expanduser(
    "~/Movies/meetily-recordings/Meeting 20_10_26_13_32_31_2026-10-20_17-32"
)

MAX_BULLETS = 3  # card.rs MAX_BULLETS
MAX_ASKS = 5  # ask_list.rs MAX_ASKS
MAX_FOR_YOU = 2  # ask_list.rs MAX_FOR_YOU
SAME_ITEM_OVERLAP = 0.6  # ask_list.rs SAME_ITEM_OVERLAP

NOT_OUTPUT = {"READY", "NONE", "SKIP"}

CREATE_TABLE = """
CREATE TABLE IF NOT EXISTS meeting_assistant_items (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    call_time REAL NOT NULL,
    shown_at TEXT NOT NULL,
    question TEXT NOT NULL DEFAULT '',
    lead TEXT NOT NULL DEFAULT '',
    bullets_json TEXT NOT NULL DEFAULT '[]',
    why TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    phase TEXT NOT NULL DEFAULT '',
    changed_lines_json TEXT NOT NULL DEFAULT '[]',
    answer TEXT NOT NULL DEFAULT '',
    answered_how TEXT NOT NULL DEFAULT '',
    answered_call_time REAL
);
CREATE INDEX IF NOT EXISTS idx_meeting_assistant_items_meeting ON meeting_assistant_items(meeting_id, position);
"""


class BackfillError(Exception):
    """A refusal or bad input with a plain message for the operator."""


# ---------------------------------------------------------------------------
# Time


def parse_ts(value):
    """ISO-8601 (Z or offset, any fraction length) to epoch milliseconds (int)."""
    v = value.strip()
    if v.endswith("Z"):
        v = v[:-1] + "+00:00"
    dt = datetime.fromisoformat(v)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return round(dt.timestamp() * 1000)


def fmt_rfc3339(ms):
    dt = datetime.fromtimestamp(ms / 1000, tz=timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (ms % 1000)


def call_time(shown_ms, start_ms):
    """Contract: max(0, (shown_at_ms - recording_start_ms) / 1000), 3 decimals."""
    return round(max(0.0, (shown_ms - start_ms) / 1000.0), 3)


def read_recording_start(folder, fallback_ms):
    """created_at of <folder>/metadata.json in epoch ms, or the fallback (session open)."""
    path = os.path.join(folder, "metadata.json")
    try:
        with open(path, encoding="utf-8") as fh:
            created = json.load(fh)["created_at"]
        v = created.strip()
        if v.endswith("Z"):
            v = v[:-1] + "+00:00"
        dt = datetime.fromisoformat(v)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return round(dt.timestamp() * 1000), True
    except (OSError, ValueError, KeyError, TypeError):
        return fallback_ms, False


# ---------------------------------------------------------------------------
# Card format (mirror of assistant/card.rs)


def _trim_h(s):
    return s.strip(" \t")


def _strip_prefix_ci(line, prefix):
    if line[: len(prefix)].upper() == prefix.upper():
        return _trim_h(line[len(prefix):])
    return None


def parse_card(text):
    """Returns dict(lead, bullets, source, is_skip, is_empty)."""
    whole = text.strip()
    if whole == "SKIP" or whole.startswith("SKIP\n") or whole == "SKIP.":
        return {"lead": "", "bullets": [], "source": "", "is_skip": True, "is_empty": True}
    card = {"lead": "", "bullets": [], "source": "", "is_skip": False, "is_empty": False}
    for raw in text.split("\n"):
        line = _trim_h(raw)
        if not line:
            continue
        body = _strip_prefix_ci(line, "LEAD:")
        if body is not None:
            card["lead"] = body
            continue
        body = _strip_prefix_ci(line, "SOURCE:")
        if body is not None:
            card["source"] = body
            continue
        if line.startswith("- ") or line == "-":
            if len(card["bullets"]) < MAX_BULLETS:
                card["bullets"].append(_trim_h(line[1:]))
        elif not card["lead"] and not card["bullets"] and not card["source"]:
            card["lead"] = line
        elif card["bullets"]:
            card["bullets"][-1] += " " + line
    card["is_empty"] = not card["lead"] and not card["bullets"] and not card["source"]
    return card


STOPWORDS = {
    "that", "this", "with", "from", "your", "yours", "they", "them", "then", "than", "have",
    "does", "will", "would", "could", "should", "which", "what", "when", "where", "into", "onto",
    "over", "under", "about", "actually", "really", "just", "only", "also", "both", "some",
    "more", "most", "much", "very", "here", "there", "these", "those", "still", "been", "being",
    "were", "weren", "wasn", "isn", "aren", "didn", "doesn",
}


def _stem(word):
    if len(word) >= 5 and word.endswith("s") and not word.endswith("ss"):
        return word[:-1]
    return word


def content_words(text):
    words = re.split(r"[^\w]+|_", text.lower())
    return {_stem(w) for w in words if len(w) >= 4 and w not in STOPWORDS}


def _number_tokens(s):
    out = set()
    for t in re.split(r"[^\w]+|_", s):
        if t and len(t) <= 2 and t.isdigit():
            out.add(t)
    return out


def similar(a, b):
    na, nb = _number_tokens(a), _number_tokens(b)
    if na and nb and not na <= nb and not nb <= na:
        return False
    ka, kb = content_words(a), content_words(b)
    if ka == kb:
        return True
    if not ka or not kb:
        return not ka and not kb
    return len(ka & kb) / min(len(ka), len(kb)) >= 0.5


def contradicts(fast, deep):
    if fast["is_empty"] or not fast["lead"] or not deep["lead"]:
        return False
    return not similar(fast["lead"], deep["lead"])


def changed_bullets(old, new):
    out = []
    for i, bullet in enumerate(new["bullets"]):
        if not (i < len(old["bullets"]) and similar(old["bullets"][i], bullet)):
            out.append(bullet)
    return out


def usable_card(card):
    """A card worth saving: not SKIP, not empty, not READY/NONE."""
    if card["is_skip"] or card["is_empty"]:
        return False
    if not card["bullets"] and not card["source"] and card["lead"].strip().upper().rstrip(".") in NOT_OUTPUT:
        return False
    return True


# ---------------------------------------------------------------------------
# Ask list format (mirror of assistant/ask_list.rs)


def _split_why(body):
    if "|" in body:
        q, rest = body.split("|", 1)
        rest = rest.strip()
        if rest.startswith("WHY:"):
            rest = rest[4:]
        return q.strip(), rest.strip()
    return body.strip(), ""


def _collapse(text):
    return " ".join(text.split())


def _split_done(body):
    """`<question> | ANSWER: <answer> | BY: call|chat`, as ask_list.rs split_done."""
    mark = "answer:"
    lower = body.lower()
    end = len(lower)
    while True:
        pos = lower.rfind(mark, 0, end)
        if pos < 0:
            return None
        if lower[:pos].rstrip().endswith("|"):
            break
        end = pos
    question = body[:pos].rstrip().rstrip("|")
    rest = body[pos + len(mark):]
    bar = rest.rfind("|")
    answer, by = rest, ""
    if bar >= 0 and rest[bar + 1:].lstrip().lower().startswith("by:"):
        answer = rest[:bar]
        by = rest[bar + 1:].lstrip()[3:].strip()
    how = "chat" if by.lower() == "chat" else "call"
    question, answer = question.strip(), answer.strip()
    if not question or not answer:
        return None
    return question, _collapse(answer), how


def parse_ask_list(raw):
    """Returns dict(asks=[(q, why)], for_you=[q], done=[(q, a, how)], none=bool) or None."""
    out = {"asks": [], "for_you": [], "done": [], "none": False}
    for line in raw.splitlines():
        line = line.strip()
        if line.startswith("- "):
            line = line[2:]
        line = line.strip()
        if line == "NONE":
            out["none"] = True
        elif line.startswith("ASK:"):
            q, why = _split_why(line[4:])
            if q and len(out["asks"]) < MAX_ASKS:
                out["asks"].append((q, why))
        elif line.startswith("DONE:"):
            d = _split_done(line[5:])
            if d:
                out["done"].append(d)
        elif line.startswith("YOU:"):
            q, _why = _split_why(line[4:])
            if q and len(out["for_you"]) < MAX_FOR_YOU:
                out["for_you"].append(q)
    if not out["asks"] and not out["for_you"] and not out["done"] and not out["none"]:
        return None
    return out


def same_item(a, b):
    """ask_list.rs same_item: used to match a DONE line to an open question."""
    if a.strip().lower() == b.strip().lower():
        return True
    wa, wb = content_words(a), content_words(b)
    longer = max(len(wa), len(wb))
    if longer == 0:
        return False
    return len(wa & wb) / longer >= SAME_ITEM_OVERLAP


def normalize_question(q):
    """Dedupe key: lowercase, collapse whitespace, trim trailing ? . !"""
    return _collapse(q.lower()).rstrip("?.! \t")


# ---------------------------------------------------------------------------
# Session transcripts


def _text_of(message):
    content = message.get("content") if isinstance(message, dict) else message
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(
            b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"
        )
    return ""


def load_turns(path):
    """Read one session file into turns: [{prompt, prompt_ts, reply, reply_ts}].

    A turn is one real user prompt (a non-empty user text; tool results carry no
    text) and the assistant text that follows it. The reply is the LAST non-empty
    assistant text before the next prompt, which is what the claude CLI's final
    `result` event holds and what the app parses. Rows repeated by uuid (forked
    sessions copy their parent's history) are read once. Also returns every
    timestamp seen, for the session-open fallback.
    """
    turns = []
    stamps = []
    seen = set()
    current = None
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            ts = row.get("timestamp")
            if ts:
                try:
                    stamps.append(parse_ts(ts))
                except ValueError:
                    pass
            kind = row.get("type")
            if kind not in ("user", "assistant"):
                continue
            uid = row.get("uuid")
            if uid:
                if uid in seen:
                    continue
                seen.add(uid)
            text = _text_of(row.get("message"))
            if kind == "user":
                if text.strip():
                    current = {"prompt": text, "prompt_ts": ts, "reply": None, "reply_ts": None}
                    turns.append(current)
            elif current is not None and text.strip():
                current["reply"] = text
                current["reply_ts"] = ts
    return turns, stamps


TRIGGER_TAIL = "\n\nAnswer in the card format, or reply SKIP."


def classify_session(turns):
    """'answer' | 'ask' | 'idle' from the prompts' shape (see lanes.rs turn_prompt, ask_lane.rs)."""
    answer = sum(1 for t in turns if "TRIGGER: " in t["prompt"])
    ask = sum(1 for t in turns if "CURRENT LIST:" in t["prompt"])
    if ask and ask >= answer:
        return "ask"
    if answer:
        return "answer"
    return "idle"


def prompt_card_request(prompt):
    """(kind, question) for an answer-lane prompt, or None."""
    if prompt.startswith("EXPLAIN this, in plain language"):
        return "explain", "Explain the last 15 seconds"
    if prompt.startswith("CATCH ME UP on this stretch"):
        return "catchup", "Catch-up"
    i = prompt.rfind("TRIGGER: ")
    if i < 0:
        return None
    body = prompt[i + len("TRIGGER: "):]
    if body.endswith(TRIGGER_TAIL):
        body = body[: -len(TRIGGER_TAIL)]
    return "answer", body.strip()


def session_model(path):
    """The model of the session's latest assistant row (fast lanes run sonnet, deep opus)."""
    last = None
    if not path:
        return None
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("type") == "assistant":
                m = (row.get("message") or {}).get("model")
                if m and m != "<synthetic>":
                    last = m
    return last


def tell_lanes(sessions):
    """Work out which answer session is fast and which is deep.

    The deep lane runs the stronger model and has read-only file tools
    (lanes.rs: deep_model, LaneTools::ReadOnly; fast_model, LaneTools::None;
    prompts/deep.md "stronger model and local files"). A tool-using session leaves
    assistant rows with tool_use blocks and no text; a no-tools session has none.
    The tie-break is reply speed: the fast lane answers within seconds of the prompt.
    Returns (fast_session, deep_session) or raises.
    """
    if len(sessions) > 2:
        raise BackfillError("expected at most 2 answer sessions, found %d" % len(sessions))
    if len(sessions) < 2:
        # One lane only (or none): a session that used tools is the deep lane.
        only = sessions[0] if sessions else None
        empty = {"path": "", "turns": [], "stamps": [], "role": "answer"}
        if only is not None and tool_use_count(only["path"]):
            return empty, only
        return (only or empty), empty
    scored = []
    for s in sessions:
        tools = tool_use_count(s["path"])
        lat = median_latency(s["turns"])
        scored.append((tools, -lat, s))
    # more tool use first, then slower replies
    scored.sort(key=lambda x: (x[0], -x[1]), reverse=True)
    deep, fast = scored[0][2], scored[1][2]
    if scored[0][0] == scored[1][0] == 0 and not median_latency(deep["turns"]) > median_latency(fast["turns"]):
        raise BackfillError("cannot tell the fast answer lane from the deep one")
    return fast, deep


def tool_use_count(path):
    n = 0
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            if '"tool_use"' not in line:
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            content = (row.get("message") or {}).get("content")
            if row.get("type") == "assistant" and isinstance(content, list):
                n += sum(1 for b in content if isinstance(b, dict) and b.get("type") == "tool_use")
    return n


def median_latency(turns):
    lat = []
    for t in turns:
        if t["reply"] is not None and "TRIGGER: " in t["prompt"]:
            lat.append(parse_ts(t["reply_ts"]) - parse_ts(t["prompt_ts"]))
    lat.sort()
    return lat[len(lat) // 2] if lat else 0


def load_sessions(sessions_dir):
    paths = sorted(glob.glob(os.path.join(sessions_dir, "*.jsonl")))
    if not paths:
        raise BackfillError("no session files in %s" % sessions_dir)
    out = []
    for p in paths:
        turns, stamps = load_turns(p)
        out.append({"path": p, "turns": turns, "stamps": stamps, "role": classify_session(turns)})
    return out


# ---------------------------------------------------------------------------
# Build items


def build_cards(fast_turns, deep_turns):
    """Answer cards. Each trigger's fast and deep replies become one item.

    Mirrors lanes.rs: the fast draft shows first (phase drafting); the deep result
    settles it (checked, or corrected with changed lines when its lead contradicts
    the draft). A deep SKIP/empty leaves the fast draft as it was. A fast SKIP with
    a deep answer settles against an empty draft, so it is checked.
    Returns list of dicts with shown_ms and the item fields.
    """
    deep_by = {}
    for t in deep_turns:
        req = prompt_card_request(t["prompt"])
        if req and t["reply"] is not None:
            deep_by.setdefault((req[0], normalize_question(req[1])), []).append(t)
    used = set()
    items = []

    def pick_deep(key, fast_ms):
        best = None
        for t in deep_by.get(key, []):
            if id(t) in used:
                continue
            d = abs(parse_ts(t["prompt_ts"]) - fast_ms)
            if best is None or d < best[0]:
                best = (d, t)
        return best[1] if best and best[0] <= 30000 else None

    keys_done = set()
    for ft in fast_turns:
        req = prompt_card_request(ft["prompt"])
        if not req:
            continue
        kind, question = req
        key = (kind, normalize_question(question))
        fast_ms = parse_ts(ft["prompt_ts"])
        dt = pick_deep(key, fast_ms)
        if dt is not None:
            used.add(id(dt))
        items.append(_settle_card(kind, question, ft, dt))
        keys_done.add(key)
    # Deep turns whose trigger has no fast turn.
    for key, lst in deep_by.items():
        for dt in lst:
            if id(dt) in used:
                continue
            kind, question = prompt_card_request(dt["prompt"])
            items.append(_settle_card(kind, question, None, dt))
    return [i for i in items if i is not None]


def _settle_card(kind, question, ft, dt):
    fast = parse_card(ft["reply"]) if ft and ft["reply"] is not None else None
    deep = parse_card(dt["reply"]) if dt and dt["reply"] is not None else None
    fast_ok = fast is not None and usable_card(fast)
    deep_ok = deep is not None and usable_card(deep)
    fast_ms = parse_ts(ft["reply_ts"]) if fast_ok else None
    deep_ms = parse_ts(dt["reply_ts"]) if deep_ok else None
    if fast_ok and deep_ok and fast_ms > deep_ms:
        fast_ok = False  # the draft arrived after the deep result: live it never showed
    if deep_ok:
        draft = fast if fast_ok else {"lead": "", "bullets": [], "source": "", "is_skip": False, "is_empty": True}
        bad = contradicts(draft, deep)
        shown = min(x for x in (fast_ms, deep_ms) if x is not None)
        return {
            "kind": kind, "shown_ms": shown, "question": question,
            "lead": deep["lead"], "bullets": deep["bullets"], "source": deep["source"],
            "phase": "corrected" if bad else "checked",
            "changed_lines": changed_bullets(draft, deep) if bad else [],
            "settled_by": "deep" + ("+fast" if fast_ok else ""),
        }
    if fast_ok:
        return {
            "kind": kind, "shown_ms": fast_ms, "question": question,
            "lead": fast["lead"], "bullets": fast["bullets"], "source": fast["source"],
            "phase": "checked" if kind in ("explain", "catchup") else "drafting",
            "changed_lines": [], "settled_by": "fast",
        }
    return None


def _carry_id(prev, question, used, new_id):
    """ask_list.rs carry_id: take the id of the first unused previous item that is the same item."""
    for item in prev:
        if item["id"] not in used and same_item(item["question"], question):
            used.add(item["id"])
            return item["id"]
    i = new_id()
    used.add(i)
    return i


def build_asks(ask_turns):
    """Ask next items, replayed the way the live app builds and saves them.

    Every ask reply of both lanes, in time order, becomes the shown list
    (ask_list.rs `build`): DONE lines are absorbed first, answered questions and
    rewordings of them are dropped, and each surviving item takes the id of the
    previous list's item it matches by `same_item`. The saved-items collector then
    keeps one item per id, merges ids whose normalized text is the same, and keeps
    the latest wording and the first-seen time.
    """
    replies = []
    for t in ask_turns:
        if t["reply"] is None:
            continue
        raw = parse_ask_list(t["reply"])
        if raw is None:  # unparseable: the app keeps the previous list
            continue
        replies.append((parse_ts(t["reply_ts"]), raw))
    replies.sort(key=lambda r: r[0])  # stable: ties keep file order

    counter = [0]

    def new_id():
        counter[0] += 1
        return "ask-%d" % counter[0]

    prev = {"asks": [], "for_you": []}
    answered = []  # {question, answer, how}
    entries = {}  # id -> collector entry
    order = []

    def see(item_id, kind, question, why, ms):
        e = entries.get(item_id)
        if e is None:
            e = {"id": item_id, "kind": kind, "shown_ms": ms, "question": question, "why": why,
                 "answer": "", "answered_how": "", "answered_ms": None, "last_ms": ms}
            entries[item_id] = e
            order.append(item_id)
        else:
            e["kind"], e["question"], e["last_ms"] = kind, question, ms
            e["why"] = why
        return e

    for ms, raw in replies:
        asks, for_you, done = list(raw["asks"]), list(raw["for_you"]), raw["done"]
        if not asks and not for_you and done and not raw["none"]:
            # DONE lines alone say nothing about the open list: it stays as shown.
            asks = [(i["question"], i["why"]) for i in prev["asks"]]
            for_you = [i["question"] for i in prev["for_you"]]
        for question, answer, how in done:  # absorb_done
            if any(same_item(a["question"], question) for a in answered):
                continue
            open_item = next((i for i in prev["asks"] + prev["for_you"] if same_item(i["question"], question)), None)
            if open_item is not None:
                item_id, question = open_item["id"], open_item["question"]
                kind = "for_you" if open_item in prev["for_you"] else "question"
            else:
                item_id, kind = new_id(), "question"
            answered.append({"question": question, "answer": answer, "how": how})
            e = see(item_id, kind, question, open_item["why"] if open_item else "", ms)
            if not e["answer"]:
                e["answer"], e["answered_how"], e["answered_ms"] = answer, how, ms

        def keep(q):
            return not any(same_item(a["question"], q) for a in answered)

        used = set()
        new_asks, new_you = [], []
        for question, why in asks:
            if keep(question):
                item = {"id": _carry_id(prev["asks"], question, used, new_id), "question": question, "why": why}
                new_asks.append(item)
                see(item["id"], "question", question, why, ms)
        for question in for_you:
            if keep(question):
                item = {"id": _carry_id(prev["for_you"], question, used, new_id), "question": question, "why": ""}
                new_you.append(item)
                see(item["id"], "for_you", question, "", ms)
        prev = {"asks": new_asks, "for_you": new_you}

    # Collector: dedupe by id (done above), then by normalized text.
    merged = {}
    out = []
    for item_id in order:
        e = entries[item_id]
        key = normalize_question(e["question"])
        m = merged.get(key)
        if m is None:
            merged[key] = e
            out.append(e)
            continue
        first = min(m["shown_ms"], e["shown_ms"])
        if e["last_ms"] >= m["last_ms"]:
            m.update(kind=e["kind"], question=e["question"], why=e["why"], last_ms=e["last_ms"])
        m["shown_ms"] = first
        if not m["answer"] and e["answer"]:
            m["answer"], m["answered_how"], m["answered_ms"] = e["answer"], e["answered_how"], e["answered_ms"]
    return out


def to_items(cards, asks, start_ms):
    """Cards and asks to contract items, in call_time order (ties keep first-seen order)."""
    rows = []
    seq = 0
    for c in cards:
        rows.append((c["shown_ms"], seq, c))
        seq += 1
    for a in asks:
        rows.append((a["shown_ms"], seq, a))
        seq += 1
    rows.sort(key=lambda r: (r[0], r[1]))
    items = []
    for pos, (ms, _s, x) in enumerate(rows):
        is_card = x["kind"] in ("answer", "ask", "explain", "catchup")
        items.append({
            "id": str(uuid.uuid4()),
            "position": pos,
            "kind": x["kind"],
            "callTime": call_time(ms, start_ms),
            "shownAt": fmt_rfc3339(ms),
            "question": x["question"],
            "lead": x["lead"] if is_card else "",
            "bullets": list(x["bullets"]) if is_card else [],
            "why": x.get("why", ""),
            "source": x["source"] if is_card else "",
            "phase": x["phase"] if is_card else "",
            "changedLines": list(x["changed_lines"]) if is_card else [],
            "answer": x.get("answer", ""),
            "answeredHow": x.get("answered_how", ""),
            "answeredCallTime": call_time(x["answered_ms"], start_ms) if x.get("answered_ms") is not None else None,
        })
    return items


def build_items(sessions_dir, folder):
    """Everything from disk to contract items. Returns (items, start_ms, notes dict)."""
    sessions = load_sessions(sessions_dir)
    answer = [s for s in sessions if s["role"] == "answer"]
    ask = [s for s in sessions if s["role"] == "ask"]
    fast, deep = tell_lanes(answer)
    all_stamps = [x for s in sessions for x in s["stamps"]]
    opened = min(all_stamps) if all_stamps else 0
    start_ms, from_meta = read_recording_start(folder, opened)
    cards = build_cards(fast["turns"], deep["turns"])
    asks = build_asks([t for s in ask for t in s["turns"]])
    items = to_items(cards, asks, start_ms)
    notes = {
        "fast": os.path.basename(fast["path"]), "deep": os.path.basename(deep["path"]),
        "ask": [os.path.basename(s["path"]) for s in ask],
        "start_from_metadata": from_meta, "start_ms": start_ms,
        "fast_models": session_model(fast["path"]), "deep_models": session_model(deep["path"]),
    }
    return items, start_ms, notes


# ---------------------------------------------------------------------------
# Output


def mmss(seconds):
    s = int(seconds)
    return "%02d:%02d" % (s // 60, s % 60)


def dry_run_table(items):
    lines = ["pos  mm:ss  kind      text"]
    for it in items:
        text = it["question"] if it["kind"] in ("question", "for_you", "answer", "ask") and it["question"] else it["lead"]
        if it["kind"] in ("explain", "catchup"):
            text = it["lead"] or it["question"]
        text = " ".join(text.split())[:80]
        extra = ""
        if it["kind"] in ("answer", "ask", "explain", "catchup"):
            extra = " [%s]" % it["phase"]
        elif it["answer"]:
            extra = " [answered:%s]" % it["answeredHow"]
        lines.append("%3d  %s  %-8s  %s%s" % (it["position"], mmss(it["callTime"]), it["kind"], text, extra))
    counts = {}
    for it in items:
        counts[it["kind"]] = counts.get(it["kind"], 0) + 1
    lines.append("")
    lines.append("counts: " + ", ".join("%s=%d" % kv for kv in sorted(counts.items())) + "  total=%d" % len(items))
    return "\n".join(lines)


def folder_payload(meeting_id, start_ms, items):
    return {
        "version": 1,
        "meetingId": meeting_id,
        "recordingStartedAt": fmt_rfc3339(start_ms),
        "items": [{
            "id": i["id"], "kind": i["kind"], "callTime": i["callTime"], "shownAt": i["shownAt"],
            "question": i["question"], "lead": i["lead"], "bullets": i["bullets"], "why": i["why"],
            "source": i["source"], "phase": i["phase"], "changedLines": i["changedLines"],
            "answer": i["answer"], "answeredHow": i["answeredHow"],
            "answeredCallTime": i["answeredCallTime"],
        } for i in items],
    }


def write_atomic(path, text):
    folder = os.path.dirname(os.path.abspath(path))
    fd, tmp = tempfile.mkstemp(prefix=".assistant.json.", suffix=".tmp", dir=folder)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(text)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def apply_items(db_path, folder, meeting_id, items, start_ms, replace=False):
    if not os.path.isfile(db_path):
        raise BackfillError("database not found: %s" % db_path)
    if not os.path.isdir(folder):
        raise BackfillError("folder not found: %s" % folder)
    if not items:
        raise BackfillError("nothing to save: the sessions held no assistant items")
    conn = sqlite3.connect(db_path)
    try:
        has_table = conn.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='meeting_assistant_items'"
        ).fetchone()
        if not has_table:
            raise BackfillError(
                "the meeting_assistant_items table is missing: run the new meetily-fork build once so it adds the table"
            )
        if not conn.execute("SELECT 1 FROM meetings WHERE id = ?", (meeting_id,)).fetchone():
            raise BackfillError("meeting %s is not in the meetings table" % meeting_id)
        have = conn.execute(
            "SELECT COUNT(*) FROM meeting_assistant_items WHERE meeting_id = ?", (meeting_id,)
        ).fetchone()[0]
        if have and not replace:
            raise BackfillError(
                "meeting %s already has %d assistant items: pass --replace to overwrite them" % (meeting_id, have)
            )
        payload = json.dumps(folder_payload(meeting_id, start_ms, items), indent=2, ensure_ascii=False) + "\n"
        conn.execute("BEGIN")
        try:
            if have:
                conn.execute("DELETE FROM meeting_assistant_items WHERE meeting_id = ?", (meeting_id,))
            conn.executemany(
                "INSERT INTO meeting_assistant_items (id, meeting_id, position, kind, call_time, shown_at,"
                " question, lead, bullets_json, why, source, phase, changed_lines_json, answer,"
                " answered_how, answered_call_time) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                [(i["id"], meeting_id, i["position"], i["kind"], i["callTime"], i["shownAt"], i["question"],
                  i["lead"], json.dumps(i["bullets"], ensure_ascii=False), i["why"], i["source"], i["phase"],
                  json.dumps(i["changedLines"], ensure_ascii=False), i["answer"], i["answeredHow"],
                  i["answeredCallTime"]) for i in items],
            )
            write_atomic(os.path.join(folder, "assistant.json"), payload)
            conn.execute("COMMIT")
        except BaseException:
            conn.execute("ROLLBACK")
            raise
    finally:
        conn.close()
    return len(items)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--sessions", default=DEFAULT_SESSIONS, help="dir holding the session .jsonl files")
    ap.add_argument("--meeting-id", default=DEFAULT_MEETING)
    ap.add_argument("--folder", default=DEFAULT_FOLDER,
                    help="meeting folder: metadata.json is read from it, assistant.json written to it on --apply")
    ap.add_argument("--db", help="meetily sqlite database (required with --apply)")
    ap.add_argument("--apply", action="store_true", help="write the rows and the folder file (default is a dry run)")
    ap.add_argument("--replace", action="store_true", help="with --apply: replace items the meeting already has")
    args = ap.parse_args(argv)
    try:
        items, start_ms, notes = build_items(args.sessions, args.folder)
        if not notes["start_from_metadata"]:
            print("warning: no readable metadata.json in %s; recording start taken as the time the "
                  "assistant session opened" % args.folder, file=sys.stderr)
        if not args.apply:
            print("DRY RUN, nothing written.")
            print("recording start: %s (%s)" % (fmt_rfc3339(start_ms),
                  "metadata.json" if notes["start_from_metadata"] else "session open"))
            print("fast answer lane: %s (%s)" % (notes["fast"], notes["fast_models"]))
            print("deep answer lane: %s (%s)" % (notes["deep"], notes["deep_models"]))
            print("ask lanes: %s" % ", ".join(notes["ask"]))
            print()
            print(dry_run_table(items))
            return 0
        if not args.db:
            raise BackfillError("--apply needs --db <path>")
        n = apply_items(args.db, args.folder, args.meeting_id, items, start_ms, args.replace)
        print("saved %d items for %s; wrote %s" % (n, args.meeting_id, os.path.join(args.folder, "assistant.json")))
        return 0
    except BackfillError as e:
        print("refused: %s" % e, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
