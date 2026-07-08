"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import {
  type Locale,
  type TranslationKey,
  getStoredLocale,
  storeLocale,
  translate,
} from "@/lib/i18n";

// ─────────────────────────────────────────────────────────────
//  Context
// ─────────────────────────────────────────────────────────────

interface LocaleCtx {
  locale: Locale;
  setLocale: (l: Locale) => void;
  t: (key: TranslationKey, vars?: Record<string, string | number>) => string;
}

const Ctx = createContext<LocaleCtx>({
  locale: "es",
  setLocale: () => {},
  t: (key) => key,
});

// ─────────────────────────────────────────────────────────────
//  Provider
// ─────────────────────────────────────────────────────────────

export function LocaleProvider({ children }: { children: React.ReactNode }) {
  // Default to "es" during SSR; swap to stored value on mount.
  const [locale, setLocaleState] = useState<Locale>("es");

  useEffect(() => {
    setLocaleState(getStoredLocale());
  }, []);

  const setLocale = useCallback((l: Locale) => {
    storeLocale(l);
    setLocaleState(l);
  }, []);

  const t = useCallback(
    (key: TranslationKey, vars?: Record<string, string | number>) =>
      translate(locale, key, vars),
    [locale]
  );

  return <Ctx.Provider value={{ locale, setLocale, t }}>{children}</Ctx.Provider>;
}

// ─────────────────────────────────────────────────────────────
//  Hook
// ─────────────────────────────────────────────────────────────

export function useLocale() {
  return useContext(Ctx);
}
