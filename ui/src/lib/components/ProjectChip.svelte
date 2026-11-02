<script lang="ts">
  import type { ProjectInfo } from '../api/types';

  // The project the server records into: its name, with the absolute path as
  // the tooltip. Both are text interpolations only (§11.1).
  let { project }: { project: ProjectInfo } = $props();
</script>

<span class="project" data-testid="topbar-project" title={project.path}>{project.name}</span>

<style>
  .project {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    max-width: 24ch;
    height: 22px;
    padding: 0 var(--s-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-pill);
  }
</style>
