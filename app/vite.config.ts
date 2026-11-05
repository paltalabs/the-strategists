import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  // ../testnet.contracts.json sits outside the app root; let the dev server read it.
  server: { fs: { allow: ['..'] } },
});
