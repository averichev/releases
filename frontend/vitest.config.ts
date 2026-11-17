import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  // Ant Design accessibility scans are CPU-heavy; serialize UI files for stable timings.
  test: { maxWorkers: 1, testTimeout: 10000, environment: 'jsdom', setupFiles: ['./src/test-setup.ts'], restoreMocks: true },
});
