#!/usr/bin/env bash
# loop.sh - Single iteration for autonomous development loop with br integration
# Usage: ./loop.sh
# Environment vars:
#   PRETTY_PRINT_DEBUG=1   Enable verbose debug output
#   PI_MODEL               Model to use (e.g., anthropic/claude-sonnet, google/gemini)

set -e

# Configuration
PROGRESS="progress.txt"
SPECS="${SPECS_DIR:-specs}"
CODE="${CODE_DIR:-src}"
BRANCH="${BRANCH:-main}"
STREAM_OUTPUT="/tmp/opencode_stream.txt"
SESSION_STATS="session_stats.jsonl"

# Export debug flag for pretty_print.py
export PRETTY_PRINT_DEBUG=${PRETTY_PRINT_DEBUG:-false}

# Debug logging function
debug_log() {
    if [ "$PRETTY_PRINT_DEBUG" = "true" ] || [ "$PRETTY_PRINT_DEBUG" = "1" ]; then
        echo -e "${DIM}[DEBUG] $*${NC}" >&2
    fi
}

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

# Initialize progress file
if [ ! -f "$PROGRESS" ]; then
    echo "# [iter]|[task_id]|[status]|[summary]|[learnings]" > "$PROGRESS"
    chmod 644 "$PROGRESS"
fi

# Get iteration count
ITER=$(grep -c "^[0-9]" "$PROGRESS" 2>/dev/null || true)
ITER=$((ITER + 1))

debug_log "Starting iteration $ITER"

# Fetch next task from br
debug_log "Fetching next task from br"
TASK_JSON=$(br ready --json 2>/dev/null | grep -v "^2026-" | jq '.[0]' 2>/dev/null || echo "null")
debug_log "Task JSON: $TASK_JSON"

if [ "$TASK_JSON" = "null" ] || [ -z "$TASK_JSON" ]; then
    echo -e "${GREEN}✓ No ready tasks in br. All done!${NC}"
    exit 0
fi

# Parse task from JSON
TASK_ID=$(echo "$TASK_JSON" | jq -r '.id // empty')
TASK_TITLE=$(echo "$TASK_JSON" | jq -r '.title // empty')
TASK_TYPE=$(echo "$TASK_JSON" | jq -r '.type // empty')
TASK_PRIORITY=$(echo "$TASK_JSON" | jq -r '.priority // empty')
TASK_DESC=$(echo "$TASK_JSON" | jq -r '.description // ""' | head -5)

if [ -z "$TASK_ID" ] || [ -z "$TASK_TITLE" ]; then
    echo -e "${YELLOW}⚠ Could not parse task from br${NC}"
    echo -e "${DIM}Raw: $TASK_JSON${NC}"
    exit 1
fi

# Mark task as in progress
debug_log "Marking task $TASK_ID as in_progress"
br update "$TASK_ID" --status in_progress 2>/dev/null || true

# Print header
echo ""
echo -e "${CYAN}╔══════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║${NC} ${BOLD}Iteration $ITER${NC}                                                     ${CYAN}║${NC}"
echo -e "${CYAN}║${NC} Task: ${BLUE}$TASK_ID${NC}  ${BOLD}$TASK_TITLE${NC}"
echo -e "${CYAN}║${NC} ${DIM}Type: $TASK_TYPE  Priority: P$TASK_PRIORITY${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════════════════════╝${NC}"

# Build context with skill
SKILL_CONTENT=""
if [ -f "skill/SKILL.md" ]; then
    SKILL_CONTENT=$(cat skill/SKILL.md)
fi

RECENT=$(tail -5 "$PROGRESS" | grep -v "^#" || echo "(first run)")

# Read PROMPT.md if it exists
PROMPT_CONTENT=""
if [ -f "PROMPT.md" ]; then
    PROMPT_CONTENT=$(cat PROMPT.md)
fi

CONTEXT=$(cat << EOF
# Active Skill: Agent Loop
$SKILL_CONTENT

---

$PROMPT_CONTENT

---

# Current Task
Task ID: $TASK_ID
Title: $TASK_TITLE
Type: $TASK_TYPE
Priority: P$TASK_PRIORITY
Description: $TASK_DESC

# Scope
Code: $CODE/
Specs: $SPECS/
Branch: $BRANCH

# Recent Progress
$RECENT

# Instructions
1. Read the PROMPT.md above for the workflow and output contract
2. Check progress.txt for learnings from previous iterations
3. Implement the current task following the guidelines
4. Run tests and fix all failures
5. End your response with DONE|<summary>|<learnings> or BLOCKED|<reason>|<tried>
EOF
)

INPUT_CHARS=${#CONTEXT}
INPUT_TOKENS_EST=$((INPUT_CHARS / 4))

echo -e "${DIM}Context: ~$INPUT_TOKENS_EST tokens${NC}"
echo ""

# Save context to file for reference
echo "$CONTEXT" > /tmp/opencode_context_$ITER.txt

echo -e "${DIM}Context saved to: /tmp/opencode_context_$ITER.txt${NC}"
echo ""
echo -e "${CYAN}→ Running pi...${NC}"
echo -e "${DIM}[DEBUG] Starting opencode at $(date)${NC}"
echo ""

START_TIME=$(date +%s)

# Run pi with JSON format and pretty printer
# Note: Pretty printer outputs to stdout, stderr goes to stream file for signal parsing
debug_log "Executing pi command (stream output: $STREAM_OUTPUT)"

# Build pi command with optional model selection
# Using --no-session for ephemeral mode (no session file created)
PI_CMD="pi --print --mode json --no-session"
if [ -n "$PI_MODEL" ]; then
    PI_CMD="pi --print --mode json --no-session --model $PI_MODEL"
fi

echo "$CONTEXT" | $PI_CMD 2>&1 | ./pretty_print.py 2> "$STREAM_OUTPUT"
PI_EXIT=$?

debug_log "pi finished with exit code: $PI_EXIT"
debug_log "Stream output saved to: $STREAM_OUTPUT"
if [ -f "$STREAM_OUTPUT" ]; then
    debug_log "Stream output size: $(wc -c < "$STREAM_OUTPUT") bytes"
    debug_log "Stream output content: $(cat "$STREAM_OUTPUT" | tr '\n' ' ' | head -c 200)..."
else
    debug_log "Stream output file not found"
fi

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Parse signals from pretty_print.py stderr output
debug_log "Parsing signals from stream output"
RESULT=$(grep -E "^(DONE|BLOCKED)\|" "$STREAM_OUTPUT" 2>/dev/null | tail -1 || true)
debug_log "Parsed RESULT: $RESULT"
RATE_LIMITED=$(grep -qi "hit your limit\|rate.limit" "$STREAM_OUTPUT" 2>/dev/null && echo "yes" || true)
CONN_ERROR=$(grep -i "connection error\|network error\|timeout" "$STREAM_OUTPUT" 2>/dev/null && echo "yes" || true)
debug_log "RATE_LIMITED=$RATE_LIMITED, CONN_ERROR=$CONN_ERROR"

# Handle rate limit
if [ "$RATE_LIMITED" = "yes" ]; then
    echo ""
    echo -e "${YELLOW}${BOLD}⏱️  Rate limit detected${NC}"
    echo "$ITER|$TASK_ID|⏸|RATE_LIMITED|pause required" >> "$PROGRESS"
    echo -e "${YELLOW}Sleeping for 60 seconds...${NC}"
    sleep 60
    exit 0
fi

# Handle connection error
if [ "$CONN_ERROR" = "yes" ]; then
    echo ""
    echo -e "${YELLOW}⚡ Connection error. Retrying in 30s.${NC}"
    echo "$ITER|$TASK_ID|⚡|CONN_ERROR|retry 30s" >> "$PROGRESS"
    sleep 30
    exit 0
fi

# Parse result and update br
debug_log "Processing result: $RESULT"
if [[ "$RESULT" == DONE* ]]; then
    SUMMARY=$(echo "$RESULT" | cut -d'|' -f2)
    LEARNINGS=$(echo "$RESULT" | cut -d'|' -f3)
    debug_log "Task completed - SUMMARY: $SUMMARY, LEARNINGS: $LEARNINGS"

    # Close the task in br
    debug_log "Closing task $TASK_ID in br"
    br close "$TASK_ID" --reason "$SUMMARY" 2>/dev/null || true

    # Add learning as comment if present
    if [ -n "$LEARNINGS" ] && [ "$LEARNINGS" != "" ]; then
        debug_log "Adding learning comment to task $TASK_ID"
        br comments add "$TASK_ID" "Learning: $LEARNINGS" 2>/dev/null || true
    fi

    echo "$ITER|$TASK_ID|✓|$SUMMARY|$LEARNINGS" >> "$PROGRESS"
    STATUS="${GREEN}✓ Completed${NC}"

elif [[ "$RESULT" == BLOCKED* ]]; then
    REASON=$(echo "$RESULT" | cut -d'|' -f2)
    TRIED=$(echo "$RESULT" | cut -d'|' -f3)
    debug_log "Task blocked - REASON: $REASON, TRIED: $TRIED"

    # Add block comment to br
    debug_log "Adding block comment to task $TASK_ID"
    br comments add "$TASK_ID" "BLOCKED: $REASON. Tried: $TRIED" 2>/dev/null || true

    echo "$ITER|$TASK_ID|✗|$REASON|$TRIED" >> "$PROGRESS"
    STATUS="${RED}✗ Blocked${NC}"
    EXIT_CODE=1

else
    # No signal - log iteration ran and continue
    debug_log "No completion signal found, marking as ran"
    echo "$ITER|$TASK_ID|→|iteration ran|no signal" >> "$PROGRESS"
    STATUS="${BLUE}→ Ran${NC}"
fi

# Sync br to git (export to JSONL)
debug_log "Syncing br to git (flush-only)"
br sync --flush-only 2>/dev/null || true
debug_log "Sync completed"

# Calculate session stats
COMPLETED=$(grep -c '|✓|' "$PROGRESS" 2>/dev/null || echo "0")
BLOCKED=$(grep -c '|✗|' "$PROGRESS" 2>/dev/null || echo "0")
RAN=$(grep -c '|→|' "$PROGRESS" 2>/dev/null || echo "0")

# Extract token counts from pretty_print output if available
TOKENS_IN=$(grep -o '📥 [0-9]*' "$STREAM_OUTPUT" 2>/dev/null | grep -o '[0-9]*' || echo "$INPUT_TOKENS_EST")
TOKENS_OUT=$(grep -o '📤 [0-9]*' "$STREAM_OUTPUT" 2>/dev/null | grep -o '[0-9]*' || echo "0")
TOOL_CALLS=$(grep -o '🔧 [0-9]*' "$STREAM_OUTPUT" 2>/dev/null | grep -o '[0-9]*' || echo "0")

# Print stats box
echo ""
echo -e "${DIM}┌─────────────────────────────────────────────────┐${NC}"
echo -e "${DIM}│${NC} ${BOLD}Iteration $ITER${NC}                                    ${DIM}│${NC}"
echo -e "${DIM}├─────────────────────────────────────────────────┤${NC}"
echo -e "${DIM}│${NC} Status:   $STATUS"
echo -e "${DIM}│${NC} Duration: ${DURATION}s"
echo -e "${DIM}│${NC} Input:    ~$INPUT_TOKENS_EST tokens"
echo -e "${DIM}├─────────────────────────────────────────────────┤${NC}"
echo -e "${DIM}│${NC} ${BOLD}Session Totals${NC}                                  ${DIM}│${NC}"
echo -e "${DIM}│${NC} ${GREEN}✓ $COMPLETED${NC}  ${RED}✗ $BLOCKED${NC}  ${BLUE}→ $RAN${NC}"
echo -e "${DIM}└─────────────────────────────────────────────────┘${NC}"

# Log session stats (JSONL format)
TASK_TITLE_ESC=$(echo "$TASK_TITLE" | sed 's/"/\\"/g' | tr -d '\n')
STATUS_CLEAN=$(echo "$STATUS" | sed 's/\x1b\[[0-9;]*m//g')
case "$STATUS_CLEAN" in
    *Completed*) STATUS_CHAR="done" ;;
    *Blocked*)   STATUS_CHAR="blocked" ;;
    *Ran*)       STATUS_CHAR="ran" ;;
    *)           STATUS_CHAR="unknown" ;;
esac

SUMMARY_CLEAN=$(echo "$RESULT" | cut -d'|' -f2 | sed 's/"/\\"/g' | tr -d '\n')
LEARNINGS_CLEAN=$(echo "$RESULT" | cut -d'|' -f3 | sed 's/"/\\"/g' | tr -d '\n')

STATS_JSON="{\"timestamp\":\"$(date -Iseconds)\",\"iteration\":$ITER,\"task_id\":\"$TASK_ID\",\"task_title\":\"$TASK_TITLE_ESC\",\"task_type\":\"$TASK_TYPE\",\"task_priority\":${TASK_PRIORITY:-0},\"status\":\"$STATUS_CHAR\",\"duration_sec\":$DURATION,\"tokens_in\":${TOKENS_IN:-0},\"tokens_out\":${TOKENS_OUT:-0},\"tokens_total\":$((${TOKENS_IN:-0} + ${TOKENS_OUT:-0})),\"tool_calls\":${TOOL_CALLS:-0},\"session_completed\":${COMPLETED:-0},\"session_blocked\":${BLOCKED:-0},\"session_rate_limited\":0,\"summary\":\"$SUMMARY_CLEAN\",\"learnings\":\"$LEARNINGS_CLEAN\"}"

echo "$STATS_JSON" >> "$SESSION_STATS"

# Exit with error code if blocked (so run.sh stops the loop)
debug_log "Iteration $ITER complete, exiting with code: ${EXIT_CODE:-0}"
if [ -n "$EXIT_CODE" ]; then
    exit $EXIT_CODE
fi

exit 0
