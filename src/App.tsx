import { Route, Routes } from "react-router-dom";

function Placeholder({ title }: { title: string }) {
  return (
    <div className="flex h-screen flex-col items-center justify-center gap-4 p-8">
      <h1 className="text-3xl font-semibold text-forest-300">Mr. Moneypenny</h1>
      <p className="text-graphite-200">{title}</p>
      <p className="text-sm text-graphite-400">
        Pre-alpha scaffold. The setup wizard, insights dashboard, and other views are not yet implemented.
      </p>
    </div>
  );
}

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<Placeholder title="Welcome." />} />
      <Route path="*" element={<Placeholder title="404 — page not found." />} />
    </Routes>
  );
}
