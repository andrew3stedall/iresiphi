"""Ticket #23: detached execution, signal checks, Ctrl-C and explicit cancellation.

Long runs use scripts/generate_fixture.py. Cancellation is cooperative and no
latency is promised (decision #13); observed latencies are printed to stderr as
evidence, and bounds asserted here are generous sanity limits, not guarantees.
"""
import os
import signal
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

import iresiphi

import python_support as support

LONG_LINES = 2000


def note(message):
    print(f"[evidence] {message}", file=sys.stderr)


class Cancellation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        root = Path(cls.directory.name)
        cls.long_root = root / "long"
        text, cls.long_bindings, cls.long_policy = support.generated(cls.long_root, LONG_LINES)
        cls.long_recipe = iresiphi.Recipe(text)
        cls.tiny_root = root / "tiny"
        cls.tiny_root.mkdir()
        support.write_inputs(cls.tiny_root)
        cls.tiny_recipe = iresiphi.Recipe(support.two_supplier_declaration())

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def tiny(self):
        return support.bindings(self.tiny_root), support.policy(self.tiny_root)

    def assertCancelledReport(self, error):
        self.assertIsInstance(error, iresiphi.RunCancelled)
        self.assertEqual(error.code, "CANCELLED")
        self.assertEqual(error.report.status, "cancelled")
        self.assertEqual(error.report_id, error.report.run_id)
        manifest = error.report.manifest
        self.assertFalse(manifest["complete"])
        self.assertEqual(error.report.outputs, [])
        self.assertTrue(any(d["code"] == "CANCELLED" for d in error.report.diagnostics))

    # -- explicit token ---------------------------------------------------------------

    def test_pre_cancelled_token(self):
        token = iresiphi.CancellationToken()
        token.cancel()
        token.cancel()
        self.assertTrue(token.cancelled)
        with self.assertRaises(iresiphi.RunCancelled) as raised:
            iresiphi.run(self.tiny_recipe, *self.tiny(), cancel=token)
        self.assertCancelledReport(raised.exception)
        self.assertNotIsInstance(raised.exception, KeyboardInterrupt)

    def test_phase_cancellation(self):
        for phase in ["parse", "join"]:
            token = iresiphi.CancellationToken()
            token.cancel_at(phase)
            with self.assertRaises(iresiphi.RunCancelled, msg=phase) as raised:
                iresiphi.run(self.tiny_recipe, *self.tiny(), cancel=token)
            self.assertCancelledReport(raised.exception)
        with self.assertRaises(iresiphi.IresiphiError) as raised:
            iresiphi.CancellationToken().cancel_at("later")
        self.assertEqual(raised.exception.code, "INVALID_ARGUMENT")

    def test_cancel_before_commit_publishes_nothing_and_after_commit_is_not_revoked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            support.write_inputs(root)
            bindings, policy = support.bindings(root), support.policy(root)
            token = iresiphi.CancellationToken()
            token.cancel_at("write")
            with self.assertRaises(iresiphi.RunCancelled):
                iresiphi.run(self.tiny_recipe, bindings, policy, cancel=token, destination=root / "a")
            self.assertFalse((root / "a").exists())
            self.assertEqual(sorted(p.name for p in root.iterdir() if p.is_dir()), [])
            token = iresiphi.CancellationToken()
            token.cancel_at("committed")
            with iresiphi.run(self.tiny_recipe, bindings, policy, cancel=token,
                              destination=root / "b") as result:
                self.assertEqual(result.status, "succeeded_with_unresolved")
                self.assertTrue(result.manifest["post_commit"]["cancel_requested"])
            self.assertTrue(token.cancelled)
            self.assertTrue((root / "b").is_dir())

    def test_cancel_from_another_thread_during_a_long_run(self):
        token = iresiphi.CancellationToken()
        timer = threading.Timer(0.5, token.cancel)
        started = time.monotonic()
        timer.start()
        try:
            with self.assertRaises(iresiphi.RunCancelled) as raised:
                iresiphi.run(self.long_recipe, self.long_bindings, self.long_policy, cancel=token)
        finally:
            timer.cancel()
        elapsed = time.monotonic() - started
        note(f"explicit cancel of a {LONG_LINES}-line run: raised {elapsed - 0.5:.3f}s after cancel()")
        self.assertCancelledReport(raised.exception)
        self.assertLess(elapsed, 15.0)

    # -- detached execution -------------------------------------------------------------

    def test_execution_releases_the_interpreter(self):
        token = iresiphi.CancellationToken()
        ticks = 0
        stop = threading.Event()

        def count():
            nonlocal ticks
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline and not stop.is_set():
                ticks += 1
            token.cancel()

        counter = threading.Thread(target=count)
        counter.start()
        try:
            with self.assertRaises(iresiphi.RunCancelled):
                iresiphi.run(self.long_recipe, self.long_bindings, self.long_policy, cancel=token)
        finally:
            stop.set()
            counter.join()
        note(f"Python thread ran {ticks} loop iterations during 1s of Rust execution")
        self.assertGreater(ticks, 10_000)

    def test_signal_handlers_run_at_most_about_100ms_apart_while_waiting(self):
        received, sent = [], []
        token = iresiphi.CancellationToken()
        previous = signal.signal(signal.SIGUSR1, lambda *_: received.append(time.monotonic()))

        def send():
            for _ in range(5):
                time.sleep(0.2)
                sent.append(time.monotonic())
                os.kill(os.getpid(), signal.SIGUSR1)
                while len(received) < len(sent) and time.monotonic() - sent[-1] < 2:
                    time.sleep(0.005)
            token.cancel()

        sender = threading.Thread(target=send)
        sender.start()
        try:
            with self.assertRaises(iresiphi.RunCancelled):
                iresiphi.run(self.long_recipe, self.long_bindings, self.long_policy, cancel=token)
        finally:
            sender.join()
            signal.signal(signal.SIGUSR1, previous)
        latencies = [r - s for s, r in zip(sent, received)]
        note("signal handler latency while detached: "
             + ", ".join(f"{1000 * latency:.1f}ms" for latency in latencies))
        self.assertEqual(len(received), 5)
        # The wait re-attaches every SIGNAL_CHECK_MS (50 ms); allow scheduling slack.
        self.assertEqual(iresiphi._native.SIGNAL_CHECK_MS, 50)
        self.assertLess(max(latencies), 0.25)

    def test_ctrl_c_cancels_the_run_and_preserves_its_report(self):
        self.assertIs(signal.getsignal(signal.SIGINT), signal.default_int_handler)
        context = iresiphi.ExecutionContext()
        sent = []

        def interrupt():
            sent.append(time.monotonic())
            os.kill(os.getpid(), signal.SIGINT)

        timer = threading.Timer(0.5, interrupt)
        timer.start()
        try:
            with self.assertRaises(KeyboardInterrupt) as raised:
                context.run(self.long_recipe, self.long_bindings, self.long_policy)
        finally:
            timer.cancel()
        latency = time.monotonic() - sent[0]
        error = raised.exception
        note(f"Ctrl-C of a {LONG_LINES}-line run: RunInterrupted raised {latency:.3f}s after SIGINT")
        self.assertIsInstance(error, iresiphi.RunInterrupted)
        self.assertCancelledReport(error)
        self.assertIsInstance(error.__cause__, KeyboardInterrupt)
        self.assertNotIsInstance(error.__cause__, iresiphi.IresiphiError)
        self.assertLess(latency, 15.0)
        # The worker finished and released the context: it accepts the next run.
        with context.run(self.tiny_recipe, *self.tiny()) as result:
            self.assertEqual(result.status, "succeeded_with_unresolved")

    def test_one_active_run_per_context(self):
        context = iresiphi.ExecutionContext()
        token = iresiphi.CancellationToken()
        outcome = {}

        def long_run():
            try:
                context.run(self.long_recipe, self.long_bindings, self.long_policy, cancel=token)
            except BaseException as error:
                outcome["error"] = error

        worker = threading.Thread(target=long_run)
        worker.start()
        try:
            time.sleep(0.5)
            with self.assertRaises(iresiphi.IresiphiError) as raised:
                context.run(self.tiny_recipe, *self.tiny())
            self.assertEqual(raised.exception.code, "RUN_BUSY")
            self.assertEqual(raised.exception.report.status, "failed")
        finally:
            token.cancel()
            worker.join()
        self.assertIsInstance(outcome["error"], iresiphi.RunCancelled)
        # A separate context is independent.
        with iresiphi.ExecutionContext().run(self.tiny_recipe, *self.tiny()) as result:
            self.assertEqual(result.status, "succeeded_with_unresolved")


if __name__ == "__main__":
    unittest.main()
