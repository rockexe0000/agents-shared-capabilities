# auto mode: the skill's `requires` command (demotool) is derived into allow.
# deny always wins; the deny below exercises deny-subtraction on a non-required command.
authorize_skill_requires: auto
rules:
  - effect: deny
    operation: run command
    scope: denied-cmd
