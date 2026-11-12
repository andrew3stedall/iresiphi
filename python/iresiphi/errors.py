"""Exceptions raised by Iresiphi.

Every exception carries the full Rust diagnostic record. ``code``, ``phase``
and ``context`` are the stable interface; ``message`` (and any engine text in it)
is human detail only.
"""

__all__ = [
    "IresiphiError",
    "HandleClosedError",
    "RunCancelled",
    "RunInterrupted",
]


class IresiphiError(Exception):
    """A structured Iresiphi failure.

    Attributes:
        code: stable machine-readable code, such as ``MISSING_BINDING``.
        phase: where it arose (``validation``, ``binding``, ``execution``, ...).
        context: optional structured context text from Rust.
        message: human message; not a stable interface.
        record: the complete diagnostic record (subject, check,
            affected_outputs, affected_count, count_status, samples,
            samples_truncated, remediation, values, report_id, ...).
        report_id: run ID of the report this failure belongs to, if any.
        report: the owning ``RunResult`` handle for run failures, if any. It keeps
            the manifest and diagnostics; close it when done.
    """

    def __init__(self, record, report=None, report_id=None):
        record = dict(record)
        self.record = record
        self.code = record.get("code")
        self.phase = record.get("phase")
        self.message = record.get("message")
        self.context = record.get("context")
        self.severity = record.get("severity")
        self.subject = record.get("subject")
        self.check = record.get("check")
        self.remediation = record.get("remediation")
        self.report = report
        self.report_id = report_id if report_id is not None else record.get("report_id")
        super().__init__(f"{self.code} [{self.phase}]: {self.message}")

    def __reduce__(self):
        return (type(self), (self.record, None, self.report_id))


class HandleClosedError(IresiphiError):
    """A closed ``Recipe``, ``Catalog`` or ``RunResult`` handle was used (``HANDLE_CLOSED``)."""


class RunCancelled(IresiphiError):
    """The run was cancelled through its ``CancellationToken``; ``report`` holds its report."""


class RunInterrupted(RunCancelled, KeyboardInterrupt):
    """A ``KeyboardInterrupt`` cancelled the run.

    It is both a ``KeyboardInterrupt`` (so ordinary interrupt handling still
    works) and a ``RunCancelled`` carrying the run report. If the interrupt
    arrived after the run had already finished, ``code`` is ``INTERRUPTED`` and
    ``report.status`` is the run's real terminal status: committed results are
    never revoked.
    """
