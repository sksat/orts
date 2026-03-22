import react from "@astrojs/react";
import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import starlightTypeDoc from "starlight-typedoc";

export default defineConfig({
  base: "/orts",
  site: "https://sksat.github.io",
  integrations: [
    react(),
    starlight({
      title: "Orts",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/sksat/orts" }],
      plugins: [
        starlightTypeDoc({
          entryPoints: ["../uneri/src/index.ts"],
          tsconfig: "../uneri/tsconfig.json",
          output: "uneri/api",
        }),
      ],
      sidebar: [
        { label: "Getting Started", slug: "getting-started" },
        {
          label: "tobari",
          collapsed: true,
          items: [{ label: "Examples", autogenerate: { directory: "tobari/examples" } }],
        },
        {
          label: "uneri",
          collapsed: true,
          items: [
            { label: "Examples", autogenerate: { directory: "uneri/examples" } },
            {
              label: "API Reference",
              collapsed: true,
              items: [
                { label: "Overview", slug: "uneri/api/readme" },
                { label: "Classes", autogenerate: { directory: "uneri/api/classes" } },
                { label: "Interfaces", autogenerate: { directory: "uneri/api/interfaces" } },
                { label: "Functions", autogenerate: { directory: "uneri/api/functions" } },
                { label: "Type Aliases", autogenerate: { directory: "uneri/api/type-aliases" } },
                { label: "Variables", autogenerate: { directory: "uneri/api/variables" } },
              ],
            },
          ],
        },
      ],
    }),
  ],
});
