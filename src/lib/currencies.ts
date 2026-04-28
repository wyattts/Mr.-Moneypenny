/**
 * Curated list of major currencies offered in Settings + the setup
 * wizard. Code is the ISO-4217 string we store; label is what the
 * dropdown displays. The user can still override with anything else
 * via the bot ("€7 espresso") for one-off entries.
 */
export interface Currency {
  code: string;
  label: string;
}

export const CURRENCIES: Currency[] = [
  { code: "USD", label: "US Dollar — $" },
  { code: "EUR", label: "Euro — €" },
  { code: "GBP", label: "British Pound — £" },
  { code: "JPY", label: "Japanese Yen — ¥" },
  { code: "CHF", label: "Swiss Franc — CHF" },
  { code: "CAD", label: "Canadian Dollar — CA$" },
  { code: "AUD", label: "Australian Dollar — A$" },
  { code: "NZD", label: "New Zealand Dollar — NZ$" },
  { code: "CNY", label: "Chinese Yuan — ¥" },
  { code: "HKD", label: "Hong Kong Dollar — HK$" },
  { code: "SGD", label: "Singapore Dollar — S$" },
  { code: "KRW", label: "Korean Won — ₩" },
  { code: "INR", label: "Indian Rupee — ₹" },
  { code: "SEK", label: "Swedish Krona — kr" },
  { code: "NOK", label: "Norwegian Krone — kr" },
  { code: "DKK", label: "Danish Krone — kr" },
  { code: "PLN", label: "Polish Złoty — zł" },
  { code: "ILS", label: "Israeli Shekel — ₪" },
  { code: "MXN", label: "Mexican Peso — MX$" },
  { code: "BRL", label: "Brazilian Real — R$" },
  { code: "ZAR", label: "South African Rand — R" },
  { code: "AED", label: "UAE Dirham — د.إ" },
  { code: "TRY", label: "Turkish Lira — ₺" },
];
