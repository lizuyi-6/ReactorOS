import { createApp } from "vue";
import { createPinia } from "pinia";
import "./design/tokens.css";
import "./design/base.css";

import App from "./App.vue";
import { router } from "./router";

// Element Plus components/styles are auto-imported on demand via
// unplugin-auto-import + unplugin-vue-components (see vite.config.ts).
// 命令式 API（ElMessage/ElMessageBox）绕过了按需样式注入，必须显式引入 CSS，
// 否则确认弹窗无遮罩无定位（渲染在视口左上角）。
import "element-plus/es/components/message/style/css";
import "element-plus/es/components/message-box/style/css";
createApp(App).use(createPinia()).use(router).mount("#app");
