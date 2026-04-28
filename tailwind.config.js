/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // Forest green primary palette — same in both themes (brand color).
        forest: {
          50: "#e8f0eb",
          100: "#c5d8cc",
          200: "#9ebda9",
          300: "#76a285",
          400: "#598e6a",
          500: "#3d7a4f",
          600: "#2d5a3d", // accent
          700: "#1f3d2b", // primary
          800: "#15291e",
          900: "#0c1812",
        },
        // Surface / text palette — wired to CSS variables so the theme
        // toggle can swap dark ↔ light by changing only what the vars
        // resolve to (see src/styles/theme.css).
        graphite: {
          50: "var(--c-graphite-50)",
          100: "var(--c-graphite-100)",
          200: "var(--c-graphite-200)",
          300: "var(--c-graphite-300)",
          400: "var(--c-graphite-400)",
          500: "var(--c-graphite-500)",
          600: "var(--c-graphite-600)",
          700: "var(--c-graphite-700)",
          800: "var(--c-graphite-800)",
          900: "var(--c-graphite-900)",
          950: "var(--c-graphite-950)",
        },
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "-apple-system", "Segoe UI", "Roboto", "sans-serif"],
        mono: ["JetBrains Mono", "ui-monospace", "SF Mono", "Menlo", "monospace"],
      },
    },
  },
  plugins: [],
};
