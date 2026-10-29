import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
export default defineConfig({
  plugins: [vue()],
  build: { rollupOptions: { output: { entryFileNames: 'assets/app.js', chunkFileNames: 'assets/[name].js', assetFileNames: 'assets/app.[ext]' } } },
  test: { environment: 'jsdom', include: ['tests/**/*.test.ts'], clearMocks: true },
})
