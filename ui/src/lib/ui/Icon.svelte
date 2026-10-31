<script lang="ts">
  import { ICONS, type IconName } from './icons';

  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<!-- Decorative: the control that hosts an icon carries the accessible name. -->
<svg
  class="icon"
  viewBox="0 0 16 16"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#each ICONS[name] as d, i (i)}
    <path {d} />
  {/each}
</svg>

<style>
  .icon {
    flex: none;
    display: inline-block;
    vertical-align: middle;
  }
</style>
