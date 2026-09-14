# fixture: MCP render axis (ADR 0008 Phase 1a)
# enables a facade stdio server + a direct http server, both defined in the
# fixture's own personal mcp/registry.yaml (unique names → catalog-independent).
skills:
mcp:
  servers:
    - name: test-stdio
    - name: test-http
hooks:
