#!/usr/bin/env python3
"""
pretty_print.py - Pretty printer for OpenCode CLI JSON stream output.

Handles: assistant messages, tool calls, tool results, errors, rate limits.
Passes completion signals (DONE|, BLOCKED|) and errors to stderr for loop.sh.

Usage: opencode run --format json "prompt" | ./pretty_print.py
"""

import sys
import json
from datetime import datetime

# ============================================================================
# COLORS & ICONS
# ============================================================================


class C:
    """ANSI color codes."""

    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    ITALIC = "\033[3m"

    RED = "\033[31m"
    GREEN = "\033[32m"
    YELLOW = "\033[33m"
    BLUE = "\033[34m"
    MAGENTA = "\033[35m"
    CYAN = "\033[36m"
    WHITE = "\033[37m"
    GRAY = "\033[90m"

    BG_RED = "\033[41m"
    BG_YELLOW = "\033[43m"
    BG_BLUE = "\033[44m"


ICONS = {
    "assistant": "🤖",
    "tool": "🔧",
    "result": "📎",
    "error": "❌",
    "rate_limit": "⏱️",
    "thinking": "💭",
    "done": "✅",
    "blocked": "⛔",
    "file": "📄",
    "search": "🔍",
    "edit": "✏️",
    "run": "▶️",
    "git": "📦",
    "test": "🧪",
    "task": "📋",
    "br": "📋",
}

TOOL_ICONS = {
    "read": "📄",
    "view": "📄",
    "cat": "📄",
    "write": "✏️",
    "edit": "✏️",
    "str_replace": "✏️",
    "create": "✏️",
    "bash": "▶️",
    "shell": "▶️",
    "execute": "▶️",
    "search": "🔍",
    "grep": "🔍",
    "rg": "🔍",
    "find": "🔍",
    "glob": "🔍",
    "git": "📦",
    "br": "📋",
    "beads": "📋",
    "test": "🧪",
    "pytest": "🧪",
    "npm": "📦",
    "node": "📦",
    "todowrite": "📋",
    "todoread": "📋",
    "question": "❓",
}

# ============================================================================
# UTILITIES
# ============================================================================


def get_tool_icon(tool_name: str) -> str:
    """Map tool name to appropriate icon."""
    name_lower = tool_name.lower()
    for key, icon in TOOL_ICONS.items():
        if key in name_lower:
            return icon
    return ICONS["tool"]


def truncate(s: str, max_len: int = 100) -> str:
    """Truncate string with ellipsis."""
    s = s.replace("\n", " ").strip()
    if len(s) <= max_len:
        return s
    return s[: max_len - 3] + "..."


def format_tool_input(tool_name: str, input_data: dict) -> str:
    """Format tool input for display."""
    if "command" in input_data:
        return truncate(input_data["command"], 80)
    # Handle both 'path' and 'file_path' (Read tool uses file_path)
    path = input_data.get("path") or input_data.get("file_path")
    if path:
        if (
            "content" in input_data
            or "file_text" in input_data
            or "newString" in input_data
        ):
            content = (
                input_data.get("content")
                or input_data.get("file_text")
                or input_data.get("newString", "")
            )
            lines = content.count("\n") + 1
            return f"{path} ({lines} lines)"
        if "oldString" in input_data or "old_str" in input_data:
            return f"editing {path}"
        return path
    if "query" in input_data or "pattern" in input_data:
        query = input_data.get("query") or input_data.get("pattern", "")
        return truncate(query, 60)
    if "questions" in input_data:
        return f"asking {len(input_data['questions'])} question(s)"
    if "prompt" in input_data:
        return truncate(input_data["prompt"], 60)

    # Fallback: show truncated JSON
    try:
        return truncate(json.dumps(input_data), 60)
    except:
        return truncate(str(input_data), 60)


def format_tool_result(result: str, max_lines: int = 12) -> str:
    """Format tool result with smart truncation."""
    if not result or not result.strip():
        return f"{C.DIM}(empty){C.RESET}"

    lines = result.split("\n")

    if len(lines) <= max_lines:
        return result

    # Show head and tail
    head_count = max_lines // 2
    tail_count = max_lines - head_count - 1

    head = lines[:head_count]
    tail = lines[-tail_count:] if tail_count > 0 else []
    omitted = len(lines) - head_count - tail_count

    parts = head + [f"{C.DIM}    ... ({omitted} lines omitted) ...{C.RESET}"] + tail
    return "\n".join(parts)


def stderr(msg: str):
    """Write to stderr for loop.sh to capture."""
    print(msg, file=sys.stderr, flush=True)


# ============================================================================
# STREAM PROCESSOR
# ============================================================================


class StreamProcessor:
    def __init__(self):
        self.current_text = ""
        self.in_text_block = False
        self.current_tool = None
        self.tool_input_buffer = ""
        self.stats = {
            "tool_calls": 0,
            "tokens_in": 0,
            "tokens_out": 0,
            "start_time": datetime.now(),
        }

    def process_line(self, line: str):
        """Process a single line of stream output."""
        line = line.strip()
        if not line:
            return

        # Check for rate limit or error in plain text
        if not line.startswith("{") and (
            "you've hit your limit" in line.lower()
            or "rate limit" in line.lower()
            or "error" in line.lower()
        ):
            print(f"{C.YELLOW}{line}{C.RESET}")
            stderr(line)
            return

        # Try to parse as JSON
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            # Not JSON - might be error or plain output
            if line and not line.startswith("{"):
                print(f"{C.DIM}{line}{C.RESET}")
            return

        self.handle_message(data)

    def handle_message(self, data: dict):
        """Handle a parsed JSON message."""
        msg_type = data.get("type", "")
        event = data.get("event", "")

        # Handle OpenCode event format
        if event:
            self.handle_event(data)
            return

        # Handle standard format
        if msg_type == "assistant":
            self.handle_assistant_message(data)
        elif msg_type == "content_block_start":
            self.handle_block_start(data.get("content_block", {}))
        elif msg_type == "content_block_delta":
            self.handle_block_delta(data.get("delta", {}))
        elif msg_type == "content_block_stop":
            self.handle_block_stop()
        elif msg_type == "tool_use":
            self.handle_tool_use(data)
        elif msg_type == "tool_result":
            self.handle_tool_result(data)
        elif msg_type == "message_start":
            usage = data.get("message", {}).get("usage", {})
            if usage:
                self.stats["tokens_in"] = usage.get("input_tokens", 0)
        elif msg_type == "message_delta":
            usage = data.get("usage", {})
            if usage:
                self.stats["tokens_out"] = usage.get("output_tokens", 0)
        elif msg_type == "error":
            self.handle_error(data)
        elif msg_type == "result":
            self.handle_result(data)

    def handle_event(self, data: dict):
        """Handle OpenCode event format."""
        event = data.get("event", "")

        if event == "text":
            text = data.get("text", "")
            print(text, end="", flush=True)
            self.current_text += text
            if not self.in_text_block:
                self.in_text_block = True

        elif event == "tool_call":
            tool_name = data.get("name", "unknown")
            tool_input = data.get("input", {})
            icon = get_tool_icon(tool_name)
            formatted = format_tool_input(tool_name, tool_input)
            print(
                f"\n{C.BLUE}{icon} {tool_name}{C.RESET} {C.DIM}-> {formatted}{C.RESET}"
            )
            self.stats["tool_calls"] += 1
            self.current_tool = {"name": tool_name}
            self.in_text_block = False

        elif event == "tool_result":
            content = data.get("content", "")
            is_error = data.get("is_error", False)
            if is_error:
                print(
                    f"{C.RED}{ICONS['error']} Error: {truncate(str(content), 200)}{C.RESET}"
                )
                stderr(f"TOOL_ERROR: {content}")
            elif content and content.strip():
                formatted = format_tool_result(str(content), max_lines=10)
                indented = "\n".join(
                    f"  {C.DIM}{line}{C.RESET}" for line in formatted.split("\n")
                )
                print(indented)

        elif event == "error":
            error_msg = data.get("message", str(data))
            print(f"\n{C.BG_RED}{C.WHITE} {ICONS['error']} ERROR {C.RESET}")
            print(f"{C.RED}{error_msg}{C.RESET}")
            stderr(f"ERROR: {error_msg}")

        elif event == "done":
            usage = data.get("usage", {})
            if usage:
                self.stats["tokens_in"] = usage.get("input_tokens", 0)
                self.stats["tokens_out"] = usage.get("output_tokens", 0)

    def handle_assistant_start(self):
        """Handle start of assistant response."""
        if not self.in_text_block:
            print(f"\n{C.GREEN}{ICONS['assistant']} Assistant{C.RESET}")
            self.in_text_block = True

    def handle_assistant_message(self, data: dict):
        """Handle complete assistant message."""
        message = data.get("message", {})
        content_blocks = message.get("content", [])

        # Print header
        print(f"\n{C.GREEN}{ICONS['assistant']} Assistant{C.RESET}")

        # Extract and print text from content blocks
        for block in content_blocks:
            block_type = block.get("type", "")

            if block_type == "text":
                text = block.get("text", "")
                print(text)
                self.current_text += text

            elif block_type == "thinking":
                thinking = block.get("thinking", "")
                print(f"{C.MAGENTA}{ICONS['thinking']} Thinking:{C.RESET}")
                print(f"{C.DIM}{thinking}{C.RESET}")

            elif block_type == "tool_use":
                tool_name = block.get("name", "unknown")
                tool_input = block.get("input", {})
                icon = get_tool_icon(tool_name)
                formatted = format_tool_input(tool_name, tool_input)
                print(
                    f"\n{C.BLUE}{icon} {tool_name}{C.RESET} {C.DIM}-> {formatted}{C.RESET}"
                )
                self.stats["tool_calls"] += 1

        # Update token stats
        usage = message.get("usage", {})
        if usage:
            self.stats["tokens_in"] = usage.get("input_tokens", 0)
            self.stats["tokens_out"] = usage.get("output_tokens", 0)

    def handle_block_start(self, block: dict):
        """Handle start of content block."""
        block_type = block.get("type", "")

        if block_type == "tool_use":
            tool_name = block.get("name", "unknown")
            icon = get_tool_icon(tool_name)
            print(f"\n{C.BLUE}{icon} {tool_name}{C.RESET}", end="")
            self.current_tool = {"name": tool_name}
            self.in_text_block = False
            self.stats["tool_calls"] += 1

        elif block_type == "thinking":
            print(f"\n{C.MAGENTA}{ICONS['thinking']} Thinking...{C.RESET}")
            self.in_text_block = False

        elif block_type == "text":
            if not self.in_text_block:
                self.in_text_block = True

    def handle_block_delta(self, delta: dict):
        """Handle content block delta."""
        delta_type = delta.get("type", "")

        if delta_type == "text_delta":
            text = delta.get("text", "")
            print(text, end="", flush=True)
            self.current_text += text

        elif delta_type == "input_json_delta":
            self.tool_input_buffer += delta.get("partial_json", "")

        elif delta_type == "thinking_delta":
            text = delta.get("thinking", "")
            print(f"{C.DIM}{text}{C.RESET}", end="", flush=True)

    def handle_block_stop(self):
        """Handle end of content block."""
        if self.in_text_block:
            print()

        if self.current_tool and self.tool_input_buffer:
            try:
                input_data = json.loads(self.tool_input_buffer)
                formatted = format_tool_input(self.current_tool["name"], input_data)
                print(f" {C.DIM}-> {formatted}{C.RESET}")
            except json.JSONDecodeError:
                pass
            self.tool_input_buffer = ""

        self.in_text_block = False
        self.current_tool = None

    def handle_tool_use(self, data: dict):
        """Handle complete tool use message."""
        tool_name = data.get("name", "unknown")
        tool_input = data.get("input", {})
        formatted = format_tool_input(tool_name, tool_input)

        if not self.tool_input_buffer:
            icon = get_tool_icon(tool_name)
            print(
                f"\n{C.BLUE}{icon} {tool_name}{C.RESET} {C.DIM}-> {formatted}{C.RESET}"
            )

    def handle_tool_result(self, data: dict):
        """Handle tool result."""
        content = data.get("content", "")
        is_error = data.get("is_error", False)

        if is_error:
            print(f"{C.RED}{ICONS['error']} Error: {truncate(content, 200)}{C.RESET}")
            stderr(f"TOOL_ERROR: {content}")
        elif content.strip():
            formatted = format_tool_result(content, max_lines=10)
            indented = "\n".join(
                f"  {C.DIM}{line}{C.RESET}" for line in formatted.split("\n")
            )
            print(indented)

    def handle_error(self, data: dict):
        """Handle error message."""
        error = data.get("error", {})
        error_msg = error.get("message", str(data))
        error_type = error.get("type", "unknown")

        print(f"\n{C.BG_RED}{C.WHITE} {ICONS['error']} ERROR: {error_type} {C.RESET}")
        print(f"{C.RED}{error_msg}{C.RESET}")
        stderr(f"ERROR: {error_msg}")

    def handle_result(self, data: dict):
        """Handle final result."""
        usage = data.get("usage", {})
        if usage:
            self.stats["tokens_in"] = usage.get("input_tokens", 0)
            self.stats["tokens_out"] = usage.get("output_tokens", 0)

    def finalize(self):
        """Print final stats and check for completion signals."""
        duration = (datetime.now() - self.stats["start_time"]).seconds

        print(f"\n{C.DIM}{'─' * 50}{C.RESET}")
        print(
            f"{C.DIM}⏱  {duration}s  │  "
            f"🔧 {self.stats['tool_calls']} tools  │  "
            f"📥 {self.stats['tokens_in']}  📤 {self.stats['tokens_out']} tokens{C.RESET}"
        )

        # Also output to stderr for loop.sh to capture
        stderr(f"🔧 {self.stats['tool_calls']}")
        stderr(f"📥 {self.stats['tokens_in']}")
        stderr(f"📤 {self.stats['tokens_out']}")

        # Check for and pass completion signals
        for line in self.current_text.split("\n"):
            line = line.strip()
            if line.startswith("DONE|"):
                print(f"{C.GREEN}{ICONS['done']} Task completed{C.RESET}")
                stderr(line)
            elif line.startswith("BLOCKED|"):
                print(f"{C.YELLOW}{ICONS['blocked']} Task blocked{C.RESET}")
                stderr(line)


# ============================================================================
# MAIN
# ============================================================================


def main():
    processor = StreamProcessor()

    try:
        for line in sys.stdin:
            processor.process_line(line)
    except KeyboardInterrupt:
        print(f"\n{C.YELLOW}Interrupted{C.RESET}")
        sys.exit(130)
    except BrokenPipeError:
        sys.exit(0)
    finally:
        processor.finalize()


if __name__ == "__main__":
    main()
