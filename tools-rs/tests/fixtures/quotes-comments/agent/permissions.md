---
x: 1
---
rules:
  - effect: allow
    operation: run command
    scope: bare-tool
  - effect: "allow"
    operation: "run command"
    scope: 'single-quoted'
  - effect: allow
    operation: run command
    scope: cfdrop  # inline comment stripped
  - effect: allow
    operation: run command
    scope: bare-tool
