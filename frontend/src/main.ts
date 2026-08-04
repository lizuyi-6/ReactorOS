import { createApp } from "vue";
import { createPinia } from "pinia";
import "./design/tokens.css";
import "./design/base.css";

import App from "./App.vue";
import { router } from "./router";

// Element Plus components/styles are auto-imported on demand via
// unplugin-auto-import + unplugin-vue-components (see vite.config.ts).
createApp(App).use(createPinia()).use(router).mount("#app");
