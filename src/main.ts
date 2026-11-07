import "@fontsource-variable/inter";
import "@fontsource/montserrat/400.css";
import "@fontsource/montserrat/500.css";
import "@fontsource/montserrat/600.css";
import "@fontsource/geist-mono/400.css";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/mods.css";

import { createApp } from "vue";
import App from "./App.vue";
import { router } from "./router";
import { slider } from "./lib/slider";

createApp(App).use(router).directive("slider", slider).mount("#app");
