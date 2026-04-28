/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // Forest green primary palette
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
        // Dark grey surface palette
        graphite: {
          50: "#f4f4f4",
          100: "#e0e0e0",
          200: "#bdbdbd",
          300: "#9e9e9e",
          400: "#707070",
          500: "#4a4a4a",
          600: "#3a3a3a",
          700: "#2a2a2a", // surface
          800: "#1f1f1f",
          900: "#1a1a1a", // background
          950: "#0d0d0d",
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
