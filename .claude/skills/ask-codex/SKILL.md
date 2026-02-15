---
name: ask-codex
description: Consult OpenAI Codex CLI (codex exec) for a second opinion on technical questions. Use for physics, math, architecture, or implementation strategy discussions.
argument-hint: "[question]"
allowed-tools:
  - Bash
---

# Ask Codex CLI

Consult the OpenAI Codex CLI for a second opinion or expert analysis.

## Usage

The user's argument is the question to ask. Run:

```bash
codex exec -c model=gpt-5.2 "<question>"
```

## Model Selection

- **`gpt-5.2`** (default): For physics, math, logic, architecture, and conceptual questions
- **`gpt-5.2-codex`**: Only for code-generation-heavy tasks (use `-c model=gpt-5.2-codex`)

Always default to `gpt-5.2` unless the question is purely about code generation.

## Guidelines

- Pass the question as a single string argument to `codex exec`
- Add "Do NOT read or modify any files. Just answer based on the information provided." to prevent Codex from exploring the codebase
- If context from the current codebase is needed, include relevant code snippets or architecture details in the question
- Report the response back to the user with your own commentary
- Set timeout to 180000ms (3 minutes) since reasoning models can take time
