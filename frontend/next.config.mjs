/** @type {import('next').NextConfig} */
const nextConfig = {
  // Tauri serves the frontend at a fixed port. The dev server runs
  // on 1420; Tauri opens a WebviewWindow pointed at it during
  // development. For production builds, `output: 'export'` generates
  // a fully static site in `frontend/out/` that Tauri bundles.
  output: "export",
  images: { unoptimized: true },
  // The Tauri Webview is its own origin; allow it explicitly.
  experimental: {
    // Server actions don't work with static export; this is a SPA.
    serverActions: { allowedOrigins: ["tauri://localhost", "http://tauri.localhost"] },
  },
  // Don't fail the build on lint warnings during the transition.
  eslint: { ignoreDuringBuilds: true },
  typescript: { ignoreBuildErrors: false },
};

export default nextConfig;
