const text = ctx.clipboard.readText();
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.clipboard.writeText(cleaned);
return { before: text.length, after: cleaned.length };
