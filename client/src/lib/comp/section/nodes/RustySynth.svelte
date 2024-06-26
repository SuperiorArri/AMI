<script>
    import { getApi } from '$lib/js/app.js';
    import InputText from '../../form/InputText.svelte';
	import BoolProp from './comp/BoolProp.svelte';
	import FileProp from './comp/FileProp.svelte';
	import NumProp from './comp/NumProp.svelte';
	import PresetProp from './comp/PresetProp.svelte';
	import VelocityMapping from './comp/VelocityMapping.svelte';

    export let id;
    export let instance;

    let reverbRoomSize;
    let reverbDamping;
    let reverbWidth;
    let reverbLevel;

    $: {instance; updateReverbFields();};

    async function changeName(newName) {
        await getApi().nodeSetName(id, newName);
    }

    async function changeEnabled(newEnabled) {
        await getApi().nodeSetEnabled(id, newEnabled);
    }

    async function changeGlobalTransposition(newEnabled) {
        await getApi().nodeSetIgnoreGlobalTransposition(id, !newEnabled);
    }

    async function changeGain(newGain) {
        await getApi().nodeSetGain(id, newGain);
    }

    async function changeTransposition(newTransposition) {
        await getApi().nodeSetTransposition(id, newTransposition);
    }

    async function changeVelocityMapping(newMapping) {
        await getApi().nodeSetVelocityMapping(id, newMapping);
    }

    async function loadFile(newFile) {
        await getApi().nodeLoadFile(id, newFile);
    }

    async function changeBankAndPreset(newBank, newPreset) {
        await getApi().nodeSetBankAndPreset(id, newBank, newPreset);
    }

    async function changeReverbActive(active) {
        await getApi().nodeSetSfReverbActive(id, active);
    }

    async function changeReverbParams() {
        await getApi().nodeSetSfReverbParams(id, {
            room_size: reverbRoomSize,
            damping: reverbDamping,
            width: reverbWidth,
            level: reverbLevel,
        });
    }

    function toDb(value) {
        return 10*Math.log10(value);
    }

    function toLin(value) {
        return Math.pow(10, value/10);
    }

    function updateReverbFields() {
        reverbRoomSize = instance.reverb.room_size;
        reverbDamping = instance.reverb.damping;
        reverbWidth = instance.reverb.width;
        reverbLevel = instance.reverb.level;
    }
</script>

<div class="contents">
    <div class="flex flex-row gap-1 items-start">
        <!-- <p class="grow-0"><InputText value={id} readonly={true} /></p>
        <p class="grow"><InputText bind:value={name} /></p> -->
        <div class="w-10 text-center"><InputText value={id} readonly={true} rounded="left" /></div>
        <div class="grow"><InputText on:change={e => changeName(e.target.value)} value={instance.name} rounded="right" /></div>
    </div>
    <BoolProp name={"Enabled"} value={instance.enabled} on:change={(e) => changeEnabled(e.detail)} />
    <BoolProp name={"Global transposition"} value={!instance.ignore_global_transposition} on:change={(e) => changeGlobalTransposition(e.detail)} />
    <NumProp name={"Gain"} value={toDb(instance.gain)} smallStep={1} largeStep={5} defaultValue={0} numDecimalPlaces={1} unit={"dB"} on:change={(e) => changeGain(toLin(e.detail))} />
    <NumProp name={"Transposition"} value={instance.transposition} smallStep={1} largeStep={12} defaultValue={0} min={-128} max={127} numDecimalPlaces={0} unit={"semit."} on:change={(e) => changeTransposition(e.detail)} />
    <VelocityMapping name={"Velocity Mapping"} value={instance.velocity_mapping} on:change={(e) => changeVelocityMapping(e.detail)} />
    <FileProp name={"SoundFont File"} value={instance.loaded_file} on:change={(e) => loadFile(e.detail)} allowedExtensions={['sf2', 'sf3']} />
    <PresetProp name={"Preset"} bank={instance.bank} preset={instance.preset} presetMap={instance.preset_map} on:change={(e) => changeBankAndPreset(e.detail.bank, e.detail.preset)} />
    <BoolProp name={"Reverb"} value={instance.reverb.active} on:change={(e) => changeReverbActive(e.detail)} />
    <NumProp name={"Reverb Room Size"} bind:value={reverbRoomSize} smallStep={0.01} largeStep={0.1} defaultValue={0.2} numDecimalPlaces={2} on:change={(e) => changeReverbParams()} />
    <NumProp name={"Reverb Damping"} bind:value={reverbDamping} smallStep={0.01} largeStep={0.1} defaultValue={0.2} numDecimalPlaces={2} on:change={(e) => changeReverbParams()} />
    <NumProp name={"Reverb Width"} bind:value={reverbWidth} smallStep={0.01} largeStep={0.1} defaultValue={0.2} numDecimalPlaces={2} on:change={(e) => changeReverbParams()} />
    <NumProp name={"Reverb Level"} bind:value={reverbLevel} smallStep={0.01} largeStep={0.1} defaultValue={0.2} numDecimalPlaces={2} on:change={(e) => changeReverbParams()} />
    <!-- <textarea>{JSON.stringify(instance, null, 4)}</textarea> -->
</div>
