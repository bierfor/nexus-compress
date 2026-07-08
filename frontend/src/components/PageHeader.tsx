"use client";

import { useLocale } from "@/components/LocaleProvider";

export function PageHeader({
  title,
  onBack,
  children,
}: {
  title: string;
  onBack?: () => void;
  children?: React.ReactNode;
}) {
  const { t } = useLocale();
  return (
    <div className="mb-8">
      {onBack && (
        <button
          onClick={onBack}
          className="text-zinc-500 hover:text-zinc-300 text-[12px] tracking-wide mb-3 flex items-center gap-1.5 transition-colors"
        >
          <span>←</span>
          <span>{t("back").replace("← ", "")}</span>
        </button>
      )}
      <h1 className="text-white text-[32px] font-semibold tracking-tight mb-2">
        {title}
      </h1>
      {children && (
        <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
          {children}
        </p>
      )}
    </div>
  );
}
