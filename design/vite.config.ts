import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // 预览工具可能通过 PORT 指定端口；默认 5178。
  server: { port: Number(process.env.PORT) || 5178 },
});
