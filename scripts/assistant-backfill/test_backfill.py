import json
import os
import sqlite3
import tempfile
import unittest

import backfill as bf

MEETING = "meeting-test"
START = "2026-10-20T17:32:32.557426+00:00"


def user(text, ts, uuid):
    return {"type": "user", "timestamp": ts, "uuid": uuid, "message": {"role": "user", "content": text}}


def assistant(text, ts, uuid, model="m", tool=False):
    content = [{"type": "tool_use", "name": "Read", "input": {}}] if tool else [{"type": "text", "text": text}]
    return {"type": "assistant", "timestamp": ts, "uuid": uuid, "message": {"model": model, "content": content}}


def trig(q):
    return "NEW TRANSCRIPT since your last turn:\nThem: hi\n\nTRIGGER: %s\n\nAnswer in the card format, or reply SKIP." % q


def ask_prompt(n):
    return "NEW TRANSCRIPT since your last turn:\nThem: %d\n\nCURRENT LIST:\n(empty)\n\nReply with the full updated list in the ASK / YOU format, or NONE." % n


def write_jsonl(path, rows):
    with open(path, "w") as fh:
        for r in rows:
            fh.write(json.dumps(r) + "\n")


def brief_rows(prefix):
    return [
        user("This is the brief. Reply with exactly the word READY.", "2026-10-20T17:32:35.329Z", prefix + "brief-u0"),
        assistant("READY", "2026-10-20T17:32:36.000Z", prefix + "brief-a0"),
        user("Standing by. Reply with exactly the word READY.", "2026-10-20T17:32:37.000Z", prefix + "brief-u1"),
        assistant("READY", "2026-10-20T17:32:38.000Z", prefix + "brief-a1"),
    ]


class Fixture:
    """A sessions dir with a fast lane, a deep lane (tool use) and one ask lane, plus a meeting folder."""

    def __init__(self, test, fast=(), deep=(), asks=()):
        self.dir = tempfile.mkdtemp()
        test.addCleanup(lambda: __import__("shutil").rmtree(self.dir, ignore_errors=True))
        self.sessions = os.path.join(self.dir, "sessions")
        self.folder = os.path.join(self.dir, "meeting")
        os.makedirs(self.sessions)
        os.makedirs(self.folder)
        with open(os.path.join(self.folder, "metadata.json"), "w") as fh:
            json.dump({"created_at": START}, fh)
        rows = brief_rows("f")
        for i, (q, reply, pts, rts) in enumerate(fast):
            rows += [user(trig(q), pts, "fu%d" % i), assistant(reply, rts, "fa%d" % i)]
        write_jsonl(os.path.join(self.sessions, "fast.jsonl"), rows)
        rows = brief_rows("d")
        for i, (q, reply, pts, rts) in enumerate(deep):
            rows += [user(trig(q), pts, "du%d" % i), assistant("", rts, "dt%d" % i, tool=True),
                     user("", rts, "dr%d" % i), assistant(reply, rts, "da%d" % i)]
        write_jsonl(os.path.join(self.sessions, "deep.jsonl"), rows)
        rows = brief_rows("k")
        for i, (reply, rts) in enumerate(asks):
            rows += [user(ask_prompt(i), rts, "ku%d" % i), assistant(reply, rts, "ka%d" % i)]
        write_jsonl(os.path.join(self.sessions, "ask.jsonl"), rows)

    def items(self):
        return bf.build_items(self.sessions, self.folder)[0]


class CallTime(unittest.TestCase):
    def test_contract_worked_example(self):
        start = bf.parse_ts("2026-10-20T17:32:32.557426+00:00")
        shown = bf.parse_ts("2026-10-20T17:34:21.030Z")
        self.assertEqual(bf.call_time(shown, start), 108.473)

    def test_never_negative(self):
        self.assertEqual(bf.call_time(1000, 5000), 0.0)

    def test_missing_metadata_falls_back_to_session_open(self):
        ms, ok = bf.read_recording_start(tempfile.mkdtemp(), 123456)
        self.assertEqual((ms, ok), (123456, False))


class Skips(unittest.TestCase):
    def test_skip_none_ready_and_empty_replies_are_not_saved(self):
        fx = Fixture(
            self,
            fast=[("q1", "SKIP", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:01.000Z"),
                  ("q2", "READY", "2026-10-20T17:35:00.000Z", "2026-10-20T17:35:01.000Z"),
                  ("q3", "NONE", "2026-10-20T17:36:00.000Z", "2026-10-20T17:36:01.000Z"),
                  ("q4", "   \n", "2026-10-20T17:37:00.000Z", "2026-10-20T17:37:01.000Z")],
            deep=[("q1", "SKIP.", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:09.000Z")],
            asks=[("NONE", "2026-10-20T17:34:00.000Z"), ("READY", "2026-10-20T17:34:05.000Z"),
                  ("DONE: nothing here | ANSWER:", "2026-10-20T17:34:06.000Z")],
        )
        self.assertEqual(fx.items(), [])

    def test_brief_and_standing_by_turns_are_ignored(self):
        fx = Fixture(self)
        self.assertEqual(fx.items(), [])


class Cards(unittest.TestCase):
    def test_fast_and_deep_merge_into_one_item_with_the_earlier_time(self):
        fx = Fixture(
            self,
            fast=[("Which proposal?", "LEAD: Proposal 3 holds\n- cheap", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.500Z")],
            deep=[("Which proposal?", "LEAD: Proposal 3 holds\n- cheap\n- safe\nSOURCE: brief",
                   "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:21.030Z")],
        )
        items = fx.items()
        self.assertEqual(len(items), 1)
        it = items[0]
        self.assertEqual(it["kind"], "answer")
        self.assertEqual(it["question"], "Which proposal?")
        self.assertEqual(it["shownAt"], "2026-10-20T17:34:02.500Z")
        self.assertEqual(it["callTime"], 89.943)
        self.assertEqual(it["phase"], "checked")
        self.assertEqual(it["bullets"], ["cheap", "safe"])
        self.assertEqual(it["source"], "brief")
        self.assertEqual(it["changedLines"], [])

    def test_deep_that_contradicts_the_draft_is_corrected_with_changed_lines(self):
        fx = Fixture(
            self,
            fast=[("Which proposal?", "LEAD: Proposal 3 holds", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.000Z")],
            deep=[("Which proposal?", "LEAD: Proposal 4 holds\n- cheaper", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:20.000Z")],
        )
        it = fx.items()[0]
        self.assertEqual(it["phase"], "corrected")
        self.assertEqual(it["changedLines"], ["cheaper"])
        self.assertEqual(it["lead"], "Proposal 4 holds")

    def test_fast_only_keeps_the_fast_content_as_a_draft(self):
        fx = Fixture(
            self,
            fast=[("q", "LEAD: Quick one\n- a", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.000Z")],
            deep=[("q", "SKIP", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:10.000Z")],
        )
        it = fx.items()[0]
        self.assertEqual((it["lead"], it["phase"]), ("Quick one", "drafting"))

    def test_deep_only_is_checked_and_uses_the_deep_time(self):
        fx = Fixture(
            self,
            fast=[("q", "SKIP", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.000Z")],
            deep=[("q", "LEAD: Deep says yes", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:21.030Z")],
        )
        it = fx.items()[0]
        self.assertEqual((it["phase"], it["callTime"]), ("checked", 108.473))

    def test_lanes_are_told_apart_even_when_file_order_swaps(self):
        fx = Fixture(
            self,
            fast=[("q", "LEAD: from fast", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.000Z")],
            deep=[("q", "LEAD: from deep", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:20.000Z")],
        )
        os.rename(os.path.join(fx.sessions, "fast.jsonl"), os.path.join(fx.sessions, "zz.jsonl"))
        self.assertEqual(fx.items()[0]["lead"], "from deep")


class Asks(unittest.TestCase):
    def test_dedupe_across_lanes_keeps_latest_wording_and_first_time(self):
        fx = Fixture(self, asks=[
            ("ASK: When do they need it? | WHY: deadline\nYOU: Should I start with notes?", "2026-10-20T17:33:07.000Z"),
            ("ASK: when do  they need it | WHY: newer reason\nASK: Who owns it?", "2026-10-20T17:34:07.000Z"),
            ("- ASK: WHEN DO THEY NEED IT?! | WHY: latest reason", "2026-10-20T17:35:07.000Z"),
        ])
        items = fx.items()
        by_q = {i["question"].lower(): i for i in items}
        self.assertEqual(len(items), 3)
        it = by_q["when do they need it?!"]
        self.assertEqual(it["kind"], "question")
        self.assertEqual(it["why"], "latest reason")
        self.assertEqual(it["shownAt"], "2026-10-20T17:33:07.000Z")
        self.assertEqual(it["callTime"], 34.443)
        self.assertEqual(by_q["should i start with notes?"]["kind"], "for_you")
        self.assertEqual([i["position"] for i in items], [0, 1, 2])

    def test_reworded_question_keeps_one_item_with_latest_wording_and_first_time(self):
        fx = Fixture(self, asks=[
            ("ASK: Which customers are raising the capacity concern? | WHY: old", "2026-10-20T17:33:07.000Z"),
            ("ASK: Which customers are raising this capacity concern now? | WHY: new", "2026-10-20T17:34:07.000Z"),
        ])
        items = fx.items()
        self.assertEqual(len(items), 1)
        it = items[0]
        self.assertEqual(it["question"], "Which customers are raising this capacity concern now?")
        self.assertEqual((it["why"], it["callTime"]), ("new", 34.443))

    def test_question_that_drops_off_and_returns_is_still_one_item(self):
        fx = Fixture(self, asks=[
            ("ASK: Who signs the purchase order? | WHY: a", "2026-10-20T17:33:07.000Z"),
            ("ASK: Where is the datacenter located? | WHY: b", "2026-10-20T17:34:07.000Z"),
            ("ASK: who signs the purchase order | WHY: c", "2026-10-20T17:35:07.000Z"),
        ])
        items = fx.items()
        self.assertEqual(sorted(i["question"] for i in items),
                         ["Where is the datacenter located?", "who signs the purchase order"])
        po = [i for i in items if "purchase" in i["question"]][0]
        self.assertEqual((po["callTime"], po["why"]), (34.443, "c"))

    def test_same_item_matcher_threshold(self):
        self.assertTrue(bf.same_item("What are the other badges under health?", "What are the other badges under health, besides workload?"))
        self.assertFalse(bf.same_item("Who signs the purchase order?", "Where is the datacenter located?"))

    def test_an_answered_line_fills_answer_how_and_time(self):
        fx = Fixture(self, asks=[
            ("ASK: What time is the meeting? | WHY: unknown", "2026-10-20T17:33:07.000Z"),
            ("DONE: What time is the meeting? | ANSWER: Nine   local | BY: chat", "2026-10-20T17:34:21.030Z"),
            ("ASK: What time is the meeting? | WHY: again", "2026-10-20T17:36:00.000Z"),
        ])
        items = fx.items()
        self.assertEqual(len(items), 1)
        it = items[0]
        self.assertEqual((it["answer"], it["answeredHow"], it["answeredCallTime"]), ("Nine local", "chat", 108.473))
        self.assertEqual(it["callTime"], 34.443)

    def test_done_without_by_means_the_call(self):
        raw = bf.parse_ask_list("DONE: Is it ready? | ANSWER: yes")
        self.assertEqual(raw["done"], [("Is it ready?", "yes", "call")])

    def test_caps_mirror_the_app(self):
        reply = "\n".join("ASK: q%d long enough? | WHY: w" % i for i in range(8)) + "\nYOU: a\nYOU: b\nYOU: c"
        raw = bf.parse_ask_list(reply)
        self.assertEqual((len(raw["asks"]), len(raw["for_you"])), (5, 2))


class Apply(unittest.TestCase):
    def make_db(self, with_table=True, with_meeting=True):
        path = os.path.join(tempfile.mkdtemp(), "m.sqlite")
        conn = sqlite3.connect(path)
        conn.execute("CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT)")
        if with_meeting:
            conn.execute("INSERT INTO meetings VALUES (?, 'x')", (MEETING,))
        if with_table:
            conn.executescript(bf.CREATE_TABLE)
        conn.commit()
        conn.close()
        return path

    def fixture(self):
        return Fixture(
            self,
            fast=[("q", "LEAD: hi\n- b", "2026-10-20T17:34:00.000Z", "2026-10-20T17:34:02.000Z")],
            asks=[("ASK: Why? | WHY: because", "2026-10-20T17:33:07.000Z")],
        )

    def test_apply_writes_rows_and_file_then_refuses_a_second_run(self):
        fx = self.fixture()
        db = self.make_db()
        items, start, _ = bf.build_items(fx.sessions, fx.folder)
        self.assertEqual(bf.apply_items(db, fx.folder, MEETING, items, start), 2)
        conn = sqlite3.connect(db)
        rows = conn.execute("SELECT position, kind, call_time FROM meeting_assistant_items ORDER BY position").fetchall()
        conn.close()
        self.assertEqual([r[1] for r in rows], ["question", "answer"])
        with open(os.path.join(fx.folder, "assistant.json")) as fh:
            doc = json.load(fh)
        self.assertEqual((doc["version"], doc["meetingId"], len(doc["items"])), (1, MEETING, 2))
        self.assertEqual(doc["recordingStartedAt"], "2026-10-20T17:32:32.557Z")
        with self.assertRaises(bf.BackfillError) as cm:
            bf.apply_items(db, fx.folder, MEETING, items, start)
        self.assertIn("--replace", str(cm.exception))
        self.assertEqual(bf.apply_items(db, fx.folder, MEETING, items, start, replace=True), 2)

    def test_refuses_without_the_table(self):
        fx = self.fixture()
        items, start, _ = bf.build_items(fx.sessions, fx.folder)
        with self.assertRaises(bf.BackfillError) as cm:
            bf.apply_items(self.make_db(with_table=False), fx.folder, MEETING, items, start)
        self.assertIn("run the new meetily-fork build once", str(cm.exception))

    def test_refuses_an_unknown_meeting(self):
        fx = self.fixture()
        items, start, _ = bf.build_items(fx.sessions, fx.folder)
        with self.assertRaises(bf.BackfillError):
            bf.apply_items(self.make_db(with_meeting=False), fx.folder, MEETING, items, start)
        self.assertFalse(os.path.exists(os.path.join(fx.folder, "assistant.json")))


if __name__ == "__main__":
    unittest.main()
