import Anthropic from "@anthropic-ai/sdk";

const client = new Anthropic(); // reads ANTHROPIC_API_KEY
const MODEL = "claude-opus-4-8";

function imageBlock(png: Buffer): Anthropic.ImageBlockParam {
  return { type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } };
}

export async function act(
  instruction: string,
  png: Buffer,
  dims: { width: number; height: number },
): Promise<{ x: number; y: number; reasoning: string }> {
  const res = await client.messages.create({
    model: MODEL,
    max_tokens: 1024,
    tools: [{
      name: "click_at",
      description: "Report the pixel to click, in screenshot coordinates.",
      input_schema: {
        type: "object",
        properties: {
          x: { type: "integer", description: `x pixel, 0..${dims.width - 1}` },
          y: { type: "integer", description: `y pixel, 0..${dims.height - 1}` },
          reasoning: { type: "string", description: "what element you're clicking and where it is" },
        },
        required: ["x", "y", "reasoning"],
      },
    }],
    tool_choice: { type: "tool", name: "click_at" },
    messages: [{
      role: "user",
      content: [
        imageBlock(png),
        { type: "text", text: `This is a ${dims.width}x${dims.height} screenshot of the StrikeHub desktop app. Return the pixel coordinates to: ${instruction}. Coordinates are in screenshot pixels with (0,0) at the top-left.` },
      ],
    }],
  });
  const block = res.content.find((b) => b.type === "tool_use");
  if (!block || block.type !== "tool_use") throw new Error("act: no tool_use in response");
  const input = block.input as { x: number; y: number; reasoning: string };
  return { x: input.x, y: input.y, reasoning: input.reasoning };
}

export async function verify(
  expectation: string,
  png: Buffer,
): Promise<{ satisfied: boolean; reasoning: string }> {
  const res = await client.messages.create({
    model: MODEL,
    max_tokens: 1024,
    tools: [{
      name: "report_state",
      description: "Report whether the expected UI state is visible.",
      input_schema: {
        type: "object",
        properties: {
          satisfied: { type: "boolean", description: "true if the expected state is clearly visible" },
          reasoning: { type: "string", description: "what you see that supports the answer" },
        },
        required: ["satisfied", "reasoning"],
      },
    }],
    tool_choice: { type: "tool", name: "report_state" },
    messages: [{
      role: "user",
      content: [
        imageBlock(png),
        { type: "text", text: `This is a screenshot of the StrikeHub desktop app. Is the following true? "${expectation}". Answer strictly from what is visible.` },
      ],
    }],
  });
  const block = res.content.find((b) => b.type === "tool_use");
  if (!block || block.type !== "tool_use") throw new Error("verify: no tool_use in response");
  const input = block.input as { satisfied: boolean; reasoning: string };
  return { satisfied: input.satisfied, reasoning: input.reasoning };
}
