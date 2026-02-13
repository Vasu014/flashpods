#!/usr/bin/env python3
"""
pretty_print.py - Pretty printer for pi CLI JSON stream output.

Handles: assistant messages, tool calls, tool results, errors, rate limits.
Passes completion signals (DONE|, BLOCKED|) and errors to stderr for loop.sh.

Usage: pi --print --mode json "prompt" | ./pretty_print.py
"""

import sys
import json
import os
from datetime import datetime

# ============================================================================
# CONFIGURATION
# ============================================================================

# Maximum lines of output to show per tool (can be overridden via PRETTY_PRINT_MAX_LINES env var)
MAX_LINES = int(os.environ.get("PRETTY_PRINT_MAX_LINES", "100"))

# Enable debug output (can be overridden via PRETTY_PRINT_DEBUG env var)
DEBUG = os.environ.get("PRETTY_PRINT_DEBUG", "false").lower() in ("true", "1", "yes")

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
    if not input_data:
        return ""
    if "command" in input_data:
        return truncate(input_data["command"], 80)
    # Handle both 'path' and 'file_path' (Read tool uses file_path)
    path = input_data.get("path") or input_data.get("file_path")
    if path:
        if (
            "content" in input_data
            or "file_text" in input_data
            or "newString" in input_data
            or "newText" in input_data
        ):
            content = (
                input_data.get("content")
                or input_data.get("file_text")
                or input_data.get("newString")
                or input_data.get("newText", "")
            )
            lines = content.count("\n") + 1
            return f"{path} ({lines} lines)"
        if "oldString" in input_data or "old_str" in input_data or "oldText" in input_data:
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
# STREAM PROCESSOR (pi Format)
# ============================================================================


class StreamProcessor:
    def __init__(self):
        self.current_text = ""
        self.in_text_block = False
        self.in_thinking_block = False
        self.current_tool = None
        self.current_tool_id = None
        self.stats = {
            "tool_calls": 0,
            "tokens_in": 0,
            "tokens_out": 0,
            "tokens_cache_read": 0,
            "start_time": datetime.now(),
        }
        if DEBUG:
            print(
                f"{C.DIM}[DEBUG] StreamProcessor initialized at {self.stats['start_time'].strftime('%H:%M:%S')}{C.RESET}",
                flush=True,
            )

    def process_line(self, line: str):
        """Process a single line of stream output."""
        line = line.strip()
        if not line:
            if DEBUG:
                print(f"{C.DIM}[DEBUG] Empty line skipped{C.RESET}", flush=True)
            return

        # Check for rate limit or error in plain text
        if not line.startswith("{"):
            lower = line.lower()
            if "you've hit your limit" in lower or "rate limit" in lower:
                print(f"{C.YELLOW}{ICONS['rate_limit']} {line}{C.RESET}")
                stderr(line)
                return
            if "error" in lower:
                print(f"{C.RED}{line}{C.RESET}")
                stderr(line)
                return
            # Other non-JSON output
            if DEBUG:
                print(
                    f"{C.DIM}[DEBUG] Non-JSON line: {line[:100]}{C.RESET}", flush=True
                )
            else:
                print(f"{C.DIM}{line}{C.RESET}")
            return

        # Try to parse as JSON
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            if DEBUG:
                print(
                    f"{C.DIM}[DEBUG] Failed to parse JSON: {line[:100]}{C.RESET}",
                    flush=True,
                )
            else:
                print(f"{C.DIM}{line}{C.RESET}")
            return

        self.handle_message(data)

    def handle_message(self, data: dict):
        """Handle a parsed JSON message (pi format)."""
        msg_type = data.get("type", "")

        # pi event types
        if msg_type == "message_update":
            self.handle_message_update(data)
        elif msg_type == "tool_execution_start":
            self.handle_tool_execution_start(data)
        elif msg_type == "tool_execution_end":
            self.handle_tool_execution_end(data)
        elif msg_type == "turn_end":
            self.handle_turn_end(data)
        elif msg_type == "agent_end":
            self.handle_agent_end(data)
        elif msg_type == "error":
            self.handle_error(data)
        elif msg_type == "session":
            # Session metadata, ignore
            pass
        elif msg_type == "agent_start":
            if DEBUG:
                print(f"{C.DIM}[DEBUG] Agent started{C.RESET}", flush=True)
        elif msg_type == "turn_start":
            if DEBUG:
                print(f"{C.DIM}[DEBUG] Turn started{C.RESET}", flush=True)
        elif msg_type == "message_start":
            # Message metadata, ignore
            pass
        elif msg_type == "message_end":
            # Message end, ignore
            pass
        else:
            # Log unknown message types for debugging
            if msg_type and DEBUG:
                print(
                    f"{C.DIM}[DEBUG] Unknown message type: {msg_type}{C.RESET}",
                    flush=True,
                )

    def handle_message_update(self, data: dict):
        """Handle message_update events from pi."""
        event = data.get("assistantMessageEvent", {})
        event_type = event.get("type", "")
        
        if event_type == "thinking_start":
            if not self.in_thinking_block:
                if DEBUG:
                    print(f"{C.DIM}[DEBUG] Thinking block started{C.RESET}", flush=True)
                self.in_thinking_block = True
                
        elif event_type == "thinking_delta":
            # We don't print thinking by default, just accumulate
            pass
            
        elif event_type == "thinking_end":
            self.in_thinking_block = False
            if DEBUG:
                print(f"{C.DIM}[DEBUG] Thinking block ended{C.RESET}", flush=True)
                
        elif event_type == "text_start":
            if not self.in_text_block:
                if DEBUG:
                    print(f"{C.DIM}[DEBUG] Text block started{C.RESET}", flush=True)
                self.in_text_block = True
                
        elif event_type == "text_delta":
            delta = event.get("delta", "")
            if delta:
                print(delta, end="", flush=True)
                self.current_text += delta
                
        elif event_type == "text_end":
            if self.in_text_block:
                print()  # End the line
                self.in_text_block = False
                
        elif event_type == "toolcall_start":
            pass  # Tool call starting, wait for delta
            
        elif event_type == "toolcall_delta":
            # Tool call arguments being streamed, we'll handle on toolcall_end
            pass
            
        elif event_type == "toolcall_end":
            tool_call = event.get("toolCall", {})
            tool_name = tool_call.get("name", "unknown")
            tool_args = tool_call.get("arguments", {})
            tool_id = tool_call.get("id", "")
            
            # End any text block
            if self.in_text_block:
                print()
                self.in_text_block = False
            
            # Print tool invocation
            icon = get_tool_icon(tool_name)
            formatted = format_tool_input(tool_name, tool_args)
            
            if formatted:
                print(f"\n{C.BLUE}{icon} {tool_name}{C.RESET} {C.DIM}-> {formatted}{C.RESET}")
            else:
                print(f"\n{C.BLUE}{icon} {tool_name}{C.RESET}")
            
            if DEBUG:
                print(f"{C.DIM}[DEBUG] Tool started: {tool_name}{C.RESET}", flush=True)
            
            self.stats["tool_calls"] += 1
            self.current_tool = tool_name
            self.current_tool_id = tool_id

    def handle_tool_execution_start(self, data: dict):
        """Handle tool execution start."""
        if DEBUG:
            tool_name = data.get("toolName", "unknown")
            print(f"{C.DIM}[DEBUG] Tool execution started: {tool_name}{C.RESET}", flush=True)

    def handle_tool_execution_end(self, data: dict):
        """Handle tool execution end with result."""
        tool_name = data.get("toolName", "unknown")
        result = data.get("result", {})
        is_error = data.get("isError", False)
        
        # Extract text from result
        output_text = ""
        if isinstance(result, dict):
            content = result.get("content", [])
            if isinstance(content, list):
                for item in content:
                    if isinstance(item, dict) and item.get("type") == "text":
                        output_text += item.get("text", "")
            else:
                output_text = str(result)
        else:
            output_text = str(result)
        
        if is_error:
            print(f"{C.RED}{ICONS['error']} Error: {truncate(output_text, 200)}{C.RESET}")
            stderr(f"TOOL_ERROR: {output_text}")
        elif output_text and output_text.strip():
            if MAX_LINES == 0:
                formatted = output_text
            else:
                formatted = format_tool_result(output_text, max_lines=MAX_LINES)
            indented = "\n".join(
                f"  {C.DIM}{line}{C.RESET}" for line in formatted.split("\n")
            )
            print(indented)
        
        self.current_tool = None
        self.current_tool_id = None

    def handle_turn_end(self, data: dict):
        """Handle turn end with token counts."""
        if self.in_text_block:
            print()
            self.in_text_block = False
        
        message = data.get("message", {})
        usage = message.get("usage", {})
        
        if usage:
            self.stats["tokens_in"] = usage.get("input", 0) + usage.get("cacheRead", 0)
            self.stats["tokens_out"] = usage.get("output", 0)
            self.stats["tokens_cache_read"] = usage.get("cacheRead", 0)
            
            if DEBUG:
                print(
                    f"{C.DIM}[DEBUG] Turn ended - tokens: in={self.stats['tokens_in']}, out={self.stats['tokens_out']}, cache={self.stats['tokens_cache_read']}{C.RESET}",
                    flush=True,
                )

    def handle_agent_end(self, data: dict):
        """Handle agent end - final summary."""
        if self.in_text_block:
            print()
            self.in_text_block = False
        
        if DEBUG:
            print(f"{C.DIM}[DEBUG] Agent ended{C.RESET}", flush=True)

    def handle_error(self, data: dict):
        """Handle error message."""
        error_msg = data.get("error", "") or data.get("message", str(data))
        
        print(f"\n{C.BG_RED}{C.WHITE} {ICONS['error']} ERROR {C.RESET}")
        print(f"{C.RED}{error_msg}{C.RESET}")
        stderr(f"ERROR: {error_msg}")

    def finalize(self):
        """Print final stats and check for completion signals."""
        # End any text block
        if self.in_text_block:
            print()

        duration = (datetime.now() - self.stats["start_time"]).seconds

        # Token summary
        total_in = self.stats["tokens_in"]
        total_out = self.stats["tokens_out"]

        if DEBUG:
            print(
                f"{C.DIM}[DEBUG] Finalizing: duration={duration}s, tools={self.stats['tool_calls']}, tokens_in={total_in}, tokens_out={total_out}{C.RESET}",
                flush=True,
            )

        print(f"\n{C.DIM}{'─' * 50}{C.RESET}")
        print(
            f"{C.DIM}⏱  {duration}s  │  "
            f"🔧 {self.stats['tool_calls']} tools  │  "
            f"📥 {total_in}  📤 {total_out} tokens{C.RESET}"
        )

        # Output to stderr for loop.sh to capture
        stderr(f"🔧 {self.stats['tool_calls']}")
        stderr(f"📥 {total_in}")
        stderr(f"📤 {total_out}")

        # Check for and pass completion signals in accumulated text
        if DEBUG:
            print(
                f"{C.DIM}[DEBUG] Checking for completion signals in accumulated text ({len(self.current_text)} chars){C.RESET}",
                flush=True,
            )

        for line in self.current_text.split("\n"):
            line = line.strip()
            if line.startswith("DONE|"):
                print(f"{C.GREEN}{ICONS['done']} Task completed{C.RESET}")
                stderr(line)
                if DEBUG:
                    print(
                        f"{C.DIM}[DEBUG] Found DONE signal: {line}{C.RESET}", flush=True
                    )
            elif line.startswith("BLOCKED|"):
                print(f"{C.YELLOW}{ICONS['blocked']} Task blocked{C.RESET}")
                stderr(line)
                if DEBUG:
                    print(
                        f"{C.DIM}[DEBUG] Found BLOCKED signal: {line}{C.RESET}",
                        flush=True,
                    )


# ============================================================================
# MAIN
# ============================================================================


def main():
    processor = StreamProcessor()

    line_count = 0
    try:
        for line in sys.stdin:
            line_count += 1
            if line_count == 1:
                # First line received
                print(
                    f"\n{C.DIM}[pretty_print] Receiving stream data...{C.RESET}",
                    flush=True,
                )
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
