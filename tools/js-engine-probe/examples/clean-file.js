const text = ctx.fs.readText(ctx.input.source);
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.fs.writeText(ctx.input.destination, cleaned);
return { destination: ctx.input.destination, characters: cleaned.length };
