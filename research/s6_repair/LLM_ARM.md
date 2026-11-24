# Study 6, arm C: LLM repair without API keys

The LLM arm does not call any model API. It runs as Claude Code subagents inside a Claude Code
session, under the user's normal Claude Code login: the same mechanism used for the blind
labelling (Study 1) and the triage (Study 2). No key is configured or stored anywhere in this
repository.

## Protocol

1. `python s6_repair/repair.py tasks` writes `data/s6/tasks.jsonl`. Each task holds only what a
   developer sees in rosfix's report: rule, message, rule explanation, file excerpt, the list of
   package.xml files. It never holds the human fix (`truth.json` is not given to the agent).
2. A Claude Code session splits the tasks into batches and starts one subagent per batch with
   the prompt below, filling in the batch path and the output path.
3. Each subagent writes `data/s6/proposals/C_claude_code_<batch>.json` in the proposal format of
   `repair.py`. Merge them into `C_claude_code.json` and run `python s6_repair/repair.py eval`.
4. The model name and date of the session are recorded in `data/s6/proposals/C_meta.json`.
   Reruns are not bit-identical (sampling, model updates); that is a stated threat to validity.

## Prompt (verbatim)

> You repair ROS 2 configuration bugs. Read the tasks in `<BATCH>` (one JSON per line: id, rule,
> message, rule_explanation, file, line, excerpt, workspace_files). For each task, propose the
> smallest edit to the workspace files that fixes the reported problem, as JSON
> `{"id", "abstain": bool, "edits": [{"file", "old", "new"}]}` where `old` is exact text copied
> from the excerpt (or empty to append) and `new` its replacement. Abstain when you are not
> confident; a wrong edit is worse than none. Use only the information in the task; do not
> open other files, do not run tools, do not browse. Write the JSON array to `<OUT>`.

## Arm D: local model (optional, offline)

Any local runtime works with the same task files. Example with Ollama (installed separately):
`ollama run qwen2.5-coder:7b` driven by a small script that sends the same prompt per task and
writes `data/s6/proposals/D_<model>.json`. It needs no account and no network after the model
download, and it is fully reproducible with a fixed seed and temperature 0.
