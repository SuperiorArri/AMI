<script>
    import { createEventDispatcher } from 'svelte';
    import Icon from '@iconify/svelte';

    export let currentSection = 'render-nodes';
    export let previousSection = currentSection;

    let dispatch = createEventDispatcher();

    export const sections = [
        ['controller', 'lucide:drum'],
        ['render-nodes', 'game-icons:grand-piano'],
        ['control-nodes', 'ant-design:control-filled'],
        ['log', 'octicon:log-16'],
        ['settings', 'material-symbols:settings'],
    ];

    function setSection(name) {
        previousSection = currentSection;
        currentSection = name;
        dispatch('section-changed');
    }
</script>

<div class="flex flex-row justify-center gap-7 text-2xl py-4 bg-slate-800 text-slate-500">
    {#each sections as [name, icon]}
        <button class:text-slate-300={currentSection===name} on:click={() => setSection(name)}><Icon icon={icon} /></button>
    {/each}
</div>
