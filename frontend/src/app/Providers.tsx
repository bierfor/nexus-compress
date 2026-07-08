"use client";

/**
 * Sprint 5.6.29 hotfix #15: client-side provider tree.
 *
 * Wraps the app in LocaleProvider + ThemeProvider. This is the
 * *only* place where the app crosses the server-component →
 * client-component boundary for state.
 */

import { LocaleProvider } from "@/components/LocaleProvider";
import { ThemeProvider } from "@/components/ThemeProvider";

export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider>
      <LocaleProvider>{children}</LocaleProvider>
    </ThemeProvider>
  );
}
