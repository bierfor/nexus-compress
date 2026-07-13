import type { Metadata } from "next";
import "./globals.css";
import { Providers } from "./Providers";

export const metadata: Metadata = {
  title: "NexusCompress",
  description: "Hybrid compressor — LZ77 + rANS + dict codec",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="es" className="dark">
      <body className="bg-bg-base text-zinc-200 antialiased min-h-screen">
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
