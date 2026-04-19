import rehypeKatex from "rehype-katex";
import rehypeStringify from "rehype-stringify";
import remarkMath from "remark-math";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";
import { describe, expect, it } from "vitest";

/**
 * Process markdown through the same remark-math + rehype-katex pipeline
 * that Astro will use. Returns the rendered HTML string.
 */
async function renderMath(markdown: string): Promise<string> {
  const result = await unified()
    .use(remarkParse)
    .use(remarkMath)
    .use(remarkRehype)
    .use(rehypeKatex)
    .use(rehypeStringify)
    .process(markdown);
  return String(result);
}

// ---------------------------------------------------------------------------
// Inline math ($...$)
// ---------------------------------------------------------------------------

describe("inline math", () => {
  it("renders $\\mu$ with KaTeX markup", async () => {
    const html = await renderMath("The parameter $\\mu$ defines gravity.");
    expect(html).toContain('class="katex"');
  });

  it("renders a fraction inline", async () => {
    const html = await renderMath("$\\frac{a}{b}$");
    expect(html).toContain('class="katex"');
  });
});

// ---------------------------------------------------------------------------
// Display math ($$...$$)
// ---------------------------------------------------------------------------

describe("display math", () => {
  it("renders display math with katex-display class", async () => {
    const md = ["$$", "\\ddot{\\mathbf{r}} = -\\frac{\\mu}{|\\mathbf{r}|^3}\\mathbf{r}", "$$"].join(
      "\n",
    );
    const html = await renderMath(md);
    expect(html).toContain('class="katex-display"');
  });
});

// ---------------------------------------------------------------------------
// Safety: dollar signs in non-math context
// ---------------------------------------------------------------------------

describe("non-math dollar signs", () => {
  it("does not parse $ inside code spans", async () => {
    const html = await renderMath("The marker `$$SOE` indicates start of ephemeris.");
    expect(html).not.toContain('class="katex"');
    expect(html).toContain("<code>");
    expect(html).toContain("$$SOE");
  });

  it("does not parse $ inside code blocks", async () => {
    const md = ["```", "echo $HOME", "```"].join("\n");
    const html = await renderMath(md);
    expect(html).not.toContain('class="katex"');
  });
});
