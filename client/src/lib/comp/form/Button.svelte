<script>
	export let rounded = 'both';

	let roundedClass = 'rounded-full';

	$: switch (rounded) {
		case 'both':
			roundedClass = 'rounded-full';
			break;
		case 'left':
			roundedClass = 'rounded-l-full';
			break;
		case 'right':
			roundedClass = 'rounded-r-full';
			break;
		default:
			roundedClass = '';
			break;
	}
</script>

<button
	{...$$restProps}
	on:click
	class="text-align-inherit inline-block w-full outline-none {roundedClass} border border-solid border-slate-700 bg-slate-800 px-2 py-1 text-sm text-slate-400 hover:text-slate-300 transition-colors ease-out read-only:outline-none">
	<slot></slot>
</button>
