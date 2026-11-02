import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// 测试环境 jsdom；echarts 本地 npm 依赖打包，禁止 CDN。
export default defineConfig({
  plugins: [react()],
  build: {
    // 保留较明显的 chunk 划分，echarts 单独成包便于核对本地打包
    rollupOptions: {
      output: {
        manualChunks: {
          echarts: ['echarts'],
          react: ['react', 'react-dom'],
        },
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
  },
});
