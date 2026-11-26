# minimal enable-list (all off) + claude.ai sync: skills + connectors turned on, plugins left off
skills:
mcp:
  servers:
hooks:
claudeai_sync:
  skills: "on"
  plugins: "off"   # explicit off == omitted
  connectors: on
