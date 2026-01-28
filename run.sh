#!/usr/bin/env bash
# run.sh - Continuous loop wrapper
# Usage: ./run.sh [max_iterations]

set -e

MAX_ITERATIONS=${1:-0}
ITERATION=0
PAUSE_BETWEEN=2

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${CYAN}${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}${BOLD}                    RALPH AGENT LOOP STARTING               ${NC}"
echo -e "${CYAN}${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${DIM}Max iterations: ${MAX_ITERATIONS:-unlimited}${NC}"
echo -e "${DIM}Progress file:  progress.txt${NC}"
echo -e "${DIM}Task manager:   br (beads rust)${NC}"
echo -e "${DIM}Started:        $(date)${NC}"
echo ""

while true; do
    ./loop.sh
    EXIT_CODE=$?

    ITERATION=$((ITERATION + 1))

    # Exit code 1 = blocked or error, stop loop
    if [ $EXIT_CODE -eq 1 ]; then
        echo ""
        echo -e "${YELLOW}Loop stopped (blocked or error)${NC}"
        break
    fi

    # Check max iterations
    if [ $MAX_ITERATIONS -gt 0 ] && [ $ITERATION -ge $MAX_ITERATIONS ]; then
        echo ""
        echo -e "${GREEN}Reached max iterations: $MAX_ITERATIONS${NC}"
        break
    fi

    # Brief pause
    echo -e "${DIM}--- Continuing in ${PAUSE_BETWEEN}s ---${NC}"
    sleep $PAUSE_BETWEEN
done

echo ""
echo -e "${CYAN}${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}${BOLD}                    SESSION COMPLETE                            ${NC}"
echo -e "${CYAN}${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${DIM}Ended: $(date)${NC}"
