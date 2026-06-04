# State Management

> How state is managed in this project.

---

## Overview

<!--
Document your project's state management conventions here.

Questions to answer:
- What state management solution do you use?
- How is local vs global state decided?
- How do you handle server state?
- What are the patterns for derived state?
-->

(To be filled by the team)

---

## State Categories

<!-- Local state, global state, server state, URL state -->

(To be filled by the team)

---

## When to Use Global State

<!-- Criteria for promoting state to global -->

(To be filled by the team)

---

## Server State

<!-- How server data is cached and synchronized -->

(To be filled by the team)

### Convention: Isolate high-frequency stream state from server-state loaders

**What**: Chat streaming state (`streamingContent`, `streamingReasoning`, tool progress, token deltas) is high-frequency UI state. It must not change props or effect dependencies for components that load server state, such as `Sidebar` and `ConversationList`.

**Why**: A token delta can arrive many times per second. If those updates recreate callback props consumed by a data-loading component, downstream `useEffect` hooks can rerun repeatedly and call commands like `chatApi.getConversations`, making the conversation list visibly flicker between loading and rendered states.

**Example**:
```tsx
const handleSidebarSelectConversation = useCallback((id: string) => {
  runAfterLeavingSettings(() => void handleSelectConversation(id))
}, [handleSelectConversation, runAfterLeavingSettings])

<Sidebar
  onSelectConversation={handleSidebarSelectConversation}
  refreshKey={sidebarRefreshKey}
/>
```

**Rule**: Pass stable callbacks (`useCallback`) and stable derived arrays (`useMemo`) into sidebar/list components. Refresh conversation lists through explicit signals such as `refreshKey`, not by letting token-level render churn alter loader dependencies.

---

## Common Mistakes

<!-- State management mistakes your team has made -->

(To be filled by the team)

- Defining inline callbacks for sidebar/list props inside `Chat` while token streaming updates live in the same parent component. This can recreate loader dependencies on every token and cause repeated list reloads.
