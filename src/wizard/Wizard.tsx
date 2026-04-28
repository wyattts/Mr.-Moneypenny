import { useWizard } from "@/lib/store";
import { WelcomeStep } from "./steps/Welcome";
import { ChooseLLMStep } from "./steps/ChooseLLM";
import { AnthropicConfigStep } from "./steps/AnthropicConfig";
import { OllamaConfigStep } from "./steps/OllamaConfig";
import { TelegramStep } from "./steps/Telegram";
import { CurrencyLocaleStep } from "./steps/CurrencyLocale";
import { CategoriesStep } from "./steps/Categories";
import { SanityStep } from "./steps/Sanity";
import { DoneStep } from "./steps/Done";

export function Wizard() {
  const step = useWizard((s) => s.step);

  switch (step) {
    case "welcome":
      return <WelcomeStep />;
    case "choose_llm":
      return <ChooseLLMStep />;
    case "anthropic":
      return <AnthropicConfigStep />;
    case "ollama":
      return <OllamaConfigStep />;
    case "telegram":
      return <TelegramStep />;
    case "locale":
      return <CurrencyLocaleStep />;
    case "categories":
      return <CategoriesStep />;
    case "sanity":
      return <SanityStep />;
    case "done":
      return <DoneStep />;
  }
}
