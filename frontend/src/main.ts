import "./styles.css";

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("missing #app root");
}

app.innerHTML = `
  <main class="dev-shell">
    <section class="panel">
      <p class="eyebrow">ReactorOS Frontend Workspace</p>
      <h1>Componentized HMI migration target</h1>
      <p>
        The production HMI still lives in <code>static/index.html</code>.
        Build future modules here, verify feature parity, then export a single
        HTML artifact for board deployment.
      </p>
      <div class="grid">
        <div><span>API Policy</span><strong>Pipeline only</strong></div>
        <div><span>Build Output</span><strong>Single HTML</strong></div>
        <div><span>Board Target</span><strong>LubanCat 2</strong></div>
      </div>
    </section>
  </main>
`;
