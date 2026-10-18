"use strict";
{
const { register, el, refs, setHTML, fmt } = Oriel;

register("network", {
  name: "Network",
  description: "Download and upload speed across all adapters",
  color: "s1",
  create(ctx) {
    const t = refs(el(`
      <article class="tile" style="--hue:var(--s1)">
        <header><h2>Network</h2>
          <span class="legend"><span><i class="key" style="--c:var(--s1)"></i> Download</span><span><i class="key" style="--c:var(--s2)"></i> Upload</span></span>
          ${ctx.go("ms-settings:network-status", "Network settings")}</header>
        <div class="pair">
          <div><span class="label">Download</span><span class="mid" data-k="down">–</span></div>
          <div><span class="label">Upload</span><span class="mid" data-k="up">–</span></div>
        </div>
        <div class="chart" data-k="chart"></div>
      </article>`));
    ctx.chart(t.chart, {
      max: "auto", floor: 64 * 1024, fmt: fmt.rate,
      series: [
        { label: "Download", color: "s1", get: s => s.net.down },
        { label: "Upload", color: "s2", get: s => s.net.up },
      ],
    });
    return {
      root: t.root,
      update(s) {
        setHTML(t.down, fmt.rateHTML(s.net.down));
        setHTML(t.up, fmt.rateHTML(s.net.up));
      },
    };
  },
});
}
