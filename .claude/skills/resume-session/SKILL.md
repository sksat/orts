---
name: resume-session
description: Resume from a previous interrupted Claude Code session. Checks conversation logs, git status, and pending work to reconstruct context.
allowed-tools:
  - Bash(readonly)
  - Read
  - Glob
  - Grep
  - Task
---

# Resume Session

Recover context from a previous interrupted Claude Code session and identify pending tasks.

## Procedure

### Step 1: Locate project conversation logs

Claude Code stores conversation logs as JSONL files in the project-specific directory under `~/.claude/projects/`. Determine the correct path by checking the current working directory mapping:

```bash
# The project dir name is derived from the absolute working directory with slashes replaced by dashes
ls -lt ~/.claude/projects/*/  # find the matching project directory
```

List the most recent log files by modification time:
```bash
ls -lt <project-log-dir>/*.jsonl | head -6
```

The current session is the most recently modified file. The **second** file is most likely the interrupted session.

### Step 2: Extract conversation context from the interrupted session

Use an Explore agent (via Task tool) to read the end of the interrupted session log and extract:
- The last several user messages (look for `"role":"human"`)
- The last assistant actions and tool calls
- Any todo lists that were active
- Whether the session ended mid-task or completed normally

**Prompt the agent with**: Read the last ~200 lines of the JSONL file. Extract user messages, active todos, and the final state of work. Identify what was incomplete when the session ended.

### Step 3: Check current repository state

In parallel with the Explore agent, check:

1. **Git status**: uncommitted changes, staged files
   ```bash
   git status
   ```

2. **Git diff summary**: what files were modified
   ```bash
   git diff --stat
   git diff --cached --stat
   ```

3. **Recent commits**: to understand what was already committed
   ```bash
   git log --oneline -10
   ```

4. **Auto memory**: check MEMORY.md for recently documented work

### Step 4: Synthesize and report

Combine findings from the log analysis and git state to produce a summary:

1. **Previous session's goal**: What was the user working on?
2. **Completed work**: What was successfully done (committed, tested)?
3. **Interrupted at**: Where exactly did the session stop?
4. **Pending tasks**: What still needs to be done?
5. **Uncommitted changes**: Are there modifications that need attention?

Present this as a concise summary to the user, then ask what they'd like to prioritize.
