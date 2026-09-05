'use strict';
/*
 * claude-code projector (STUB — to be completed).
 * Target: ~/.claude.json `mcpServers` (or a project .mcp.json).
 * SAFETY: ~/.claude.json holds live credentials/session state — the completed
 * projector must MERGE (read-modify-write) the managed server keys only, never
 * rewrite the file wholesale. Until then, emit the intended snippet to
 * .sync-output/claude-code.mcp.json for manual/reviewed merge.
 */
module.exports.projectMcp = function projectMcp(/* servers, home */) {
  throw new Error('claude-code MCP projector not implemented — see handoff');
};
