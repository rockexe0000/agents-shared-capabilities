---
title: fixture
authorize_skill_requires: explicit
---
authorize_skill_requires: explicit
rules:
  - effect: "allow"
    operation: "run command"
    scope: "zebra"
  - effect: "allow"
    operation: "run command"
    scope: "alpha"
  - effect: "deny"
    operation: "run command"
    scope: "zebra"
  - effect: "ask"
    operation: "run command"
    scope: "curl"
  - effect: "allow"
    operation: "push branches + open PRs"
    scope: "repo x"
