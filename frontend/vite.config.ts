import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': { target: process.env.VERUNA_DEV_API_TARGET ?? 'http://127.0.0.1:8080' },
    },
  },
  build: {
    sourcemap: false,
    rolldownOptions: {
      output: {
        codeSplitting: {
          // Keep framework code cached independently from application changes.
          groups: [{ name: 'react', test: /node_modules\/(react|react-dom|react-router|react-router-dom|scheduler)\// }],
        },
      },
    },
  },
});
