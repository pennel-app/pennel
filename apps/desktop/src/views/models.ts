import { h } from "../dom";
import type { Context } from "../main";
import { inferenceModelsView } from "./inference";

export async function modelsView(ctx: Context): Promise<HTMLElement> {
  return h("div", { class: "models-settings" }, ...await inferenceModelsView(ctx));
}
