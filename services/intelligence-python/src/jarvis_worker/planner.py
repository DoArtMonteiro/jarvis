"""Planning: turn a goal into a list of typed tool calls.

``StubPlanner`` is a deterministic stand-in for a future model-backed planner. It looks for a few
keywords and produces the same plan for the same goal, which keeps the whole system testable
without a model. Whatever a planner proposes is only a proposal: the Core validates every
request, derives the capabilities it needs and applies policy before anything runs.
"""

import re
import unicodedata
from dataclasses import dataclass
from typing import Protocol

from jarvis_worker.protocol import (
    TOOL_DESKTOP_WRITE_CLIPBOARD_TEXT,
    TOOL_READ_FIXTURE,
    TOOL_SESSION_AGENT_HEALTHCHECK,
    TOOL_SYSTEM_INFO,
    TOOL_WRITE_FILE,
    JsonValue,
)


@dataclass(frozen=True, slots=True)
class PlannedCall:
    """One tool call a planner proposes."""

    tool: str
    args: dict[str, JsonValue]


class Planner(Protocol):
    """Turns a natural-language goal into the tool calls that would serve it."""

    def plan(self, goal: str) -> list[PlannedCall]:
        """Return the calls for ``goal`` in the order they should run; empty if none apply."""
        ...


# One left-to-right scan. ``read <target>`` consumes its target, so a word inside it (as in
# ``read system.txt``) is not mistaken for a topic, and ``write <path>: <text>`` consumes the
# rest of the goal.
_SCAN = re.compile(
    r"\bwrite\s+(?P<write_path>[^\s:]+):\s?(?P<content>.*)"
    r"|\bread\s+(?P<target>\S+)"
    r"|\bhealthcheck\s+(?P<check_label>\S+)"
    r'|\bcopy\s+"(?P<copy_text>.*)"'
    r"|\b(?P<topic>system|runtime|machine|environment)\b",
    re.IGNORECASE,
)
_TRAILING_PUNCTUATION = ",.;:!?"

# A conservative subset of what the Core accepts as clipboard text (it re-checks, and is
# the authority): 1 to 1024 characters, each a newline, tab, space, printable ASCII, a letter
# or digit of any script, or common typographic punctuation. Anything else, including format,
# separator and private-use characters and the Hangul fillers, yields no call.
_CLIPBOARD_TEXT_MAX_CHARS = 1024
_HANGUL_FILLERS = frozenset("ᅟᅠㅤﾠ")
_CLIPBOARD_PUNCTUATION_RANGES = (
    (0x00A1, 0x00AC),
    (0x00AE, 0x00BF),
    (0x00D7, 0x00D7),
    (0x00F7, 0x00F7),
    (0x2010, 0x2015),
    (0x2018, 0x2022),
    (0x2026, 0x2026),
    (0x2030, 0x2030),
    (0x2039, 0x203A),
    (0x20AC, 0x20AC),
    (0x2122, 0x2122),
)


def _is_plain(char: str) -> bool:
    if char in "\n\t " or "!" <= char <= "~":
        return True
    if char in _HANGUL_FILLERS:
        return False
    if unicodedata.category(char)[0] in "LN":
        return True
    return any(low <= ord(char) <= high for low, high in _CLIPBOARD_PUNCTUATION_RANGES)


def _is_clipboard_text(text: str) -> bool:
    if not 1 <= len(text) <= _CLIPBOARD_TEXT_MAX_CHARS:
        return False
    return all(_is_plain(char) for char in text)


class StubPlanner:
    """Keyword rules, applied in order of appearance in the goal (case-insensitive).

    - The first of the words ``system``, ``runtime``, ``machine`` or ``environment`` yields one
      ``system.info`` call; later occurrences add nothing.
    - Each ``read <target>`` yields a ``filesystem.read_fixture`` call. The target is the next
      whitespace-delimited word without trailing ``,.;:!?``. It is passed through unchanged even
      if it looks dangerous: judging paths is the Core's job. A target that is empty after
      stripping is skipped.
    - ``write <path>: <text>`` yields one ``workspace.write_file`` call that writes ``<text>``
      (everything after the colon and one optional space, to the end of the goal) to
      ``<path>``. Because it takes the rest of the goal, a write is always the last call. Policy
      decides whether a person must approve it; the planner does not know or care.
    - ``healthcheck <label>`` yields one ``session_agent.healthcheck`` call asking the Session
      Agent to echo ``<label>`` (the next word, without trailing ``,.;:!?``). It touches no
      desktop data; it exercises the approval and Session Agent path end to end.
    - ``copy "<text>"`` yields one ``desktop.write_clipboard_text`` call for ``<text>``: everything
      between the first double quote after ``copy`` and the last double quote of the goal, so
      the text may itself contain quotes. Empty or over-long text, and text with any character
      outside letters, digits, common punctuation, spaces and tabs, yields no call. The planner
      never reads the clipboard.
    - A goal that matches nothing yields an empty plan.
    """

    def plan(self, goal: str) -> list[PlannedCall]:
        calls: list[PlannedCall] = []
        wants_system_info = False
        for match in _SCAN.finditer(goal):
            target = match.group("target")
            write_path = match.group("write_path")
            if write_path is not None:
                calls.append(
                    PlannedCall(TOOL_WRITE_FILE, {"path": write_path, "content": match["content"]})
                )
            elif (check_label := match.group("check_label")) is not None:
                label = check_label.rstrip(_TRAILING_PUNCTUATION)
                if label:
                    calls.append(PlannedCall(TOOL_SESSION_AGENT_HEALTHCHECK, {"label": label}))
            elif (copy_text := match.group("copy_text")) is not None:
                if _is_clipboard_text(copy_text):
                    calls.append(
                        PlannedCall(TOOL_DESKTOP_WRITE_CLIPBOARD_TEXT, {"text": copy_text})
                    )
            elif target is not None:
                path = target.rstrip(_TRAILING_PUNCTUATION)
                if path:
                    calls.append(PlannedCall(TOOL_READ_FIXTURE, {"path": path}))
            elif not wants_system_info:
                wants_system_info = True
                calls.append(PlannedCall(TOOL_SYSTEM_INFO, {}))
        return calls
