# Optimize usage stats request logs pagination

## Goal

Make the Usage settings page stay responsive as local model request logs grow. The request log detail table should no longer create a long scroll-heavy settings page, and the backend should avoid unnecessary work for date-limited ranges.

## What I Already Know

- The user observed that the Usage > request logs area has no visible pagination and makes the settings page scroll poorly.
- Current frontend calls `usage_get_stats` with `limit: 120`, so the logs are bounded, but the UI only shows the first slice and has no navigation.
- Current backend supports `limit` and `offset`, but `usage_get_stats` reads all usage jsonl files before filtering and aggregating.
- Usage files are stored monthly as `usage-YYYY-MM.jsonl`.

## Requirements

- Add visible pagination controls for request logs.
- Keep per-page rendered rows small enough to reduce settings-page scroll and table layout cost.
- Reset to the first page when range, source, status, provider search, or model search changes.
- Debounce provider/model search input before calling the backend.
- Preserve existing overview, trend, provider stats, model stats, and clear/refresh behavior.
- Reduce backend disk/parse work for finite date ranges by skipping usage files that cannot contain matching records.

## Acceptance Criteria

- [ ] Request logs show a page size smaller than the previous 120-row render.
- [ ] User can navigate previous/next pages when more logs match the active filters.
- [ ] Filter/search changes return the logs to page 1.
- [ ] `usage_get_stats` still returns summary/trend/group stats for the filtered range.
- [ ] 7d/30d/90d stats do not read usage files from months wholly before the selected range start.
- [ ] TypeScript typecheck and targeted Rust tests pass.

## Out of Scope

- Splitting overview/group stats and log pagination into separate Tauri commands.
- Persistent user-configurable retention policy.
- Virtualized table dependency.
- Changing the usage storage format.

## Technical Notes

- Frontend: `src/settings/UsageStatsPanel.tsx`
- Tauri bridge types: `src/api/tauri.ts`
- Backend command/storage: `src-tauri/src/usage.rs`
