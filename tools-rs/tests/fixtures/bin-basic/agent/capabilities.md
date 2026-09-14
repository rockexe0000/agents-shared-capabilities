# fixture: bin axis + requires closure (ADR 0008 Phase 1b)
# enables a personal skill whose SKILL.md `requires` a personal bin tool → drives
# bin-install.tsv and (with auto mode) authz allow + authz-suggest.txt.
skills:
  - name: demo-skill
    source: personal
mcp:
  servers:
hooks:
