import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
export default defineConfig({ plugins: [react()], server: { host: '127.0.0.1', proxy: { '/auth':'http://127.0.0.1:4318', '/v1': 'http://127.0.0.1:4318' } } });
