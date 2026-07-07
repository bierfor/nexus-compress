/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Dark cyberpunk palette — the heart of the visual identity.
        bg: {
          base: "#0a0a0c", // app background (deepest black)
          card: "#121215", // card / panel background
          surface: "#1a1a1f", // raised surface (buttons, inputs)
          border: "#2a2a30", // subtle border
        },
        // Two accent ramps: cyan (default) + magenta (secondary).
        // Both are bright enough to read on a black background and
        // glow under Tailwind's `text-shadow` filters.
        cyan: {
          400: "#5ff5ff",
          500: "#00f0ff", // primary accent — neon cyan
          600: "#00c0cc",
        },
        magenta: {
          400: "#ff66dd",
          500: "#ff00aa", // secondary — hot magenta
          600: "#cc0088",
        },
        matrix: {
          400: "#66ff99",
          500: "#00ff66", // "matrix green" — telemetry / live data
          600: "#00cc52",
        },
        // Status colors
        ok: "#00ff66",
        warn: "#ffaa00",
        err: "#ff3344",
      },
      fontFamily: {
        mono: ["ui-monospace", "SF Mono", "Menlo", "Monaco", "monospace"],
        sans: ["ui-sans-serif", "system-ui", "sans-serif"],
      },
      // The dashed/dotted border style is the visual fingerprint of
      // the dropzone. Tailwind 3.4 has `border-dashed` / `border-dotted`
      // built in; we extend it with a slower pulse animation.
      keyframes: {
        "border-pulse": {
          "0%, 100%": { borderColor: "#00f0ff", opacity: "0.6" },
          "50%": { borderColor: "#00f0ff", opacity: "1.0" },
        },
        "glow": {
          "0%, 100%": { textShadow: "0 0 4px #00f0ff, 0 0 8px #00f0ff" },
          "50%": { textShadow: "0 0 8px #00f0ff, 0 0 16px #00f0ff" },
        },
      },
      animation: {
        "border-pulse": "border-pulse 2s ease-in-out infinite",
        glow: "glow 1.5s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
