import { h } from "../dom";
import type { Context } from "../main";
import { inferenceModelsView } from "./inference";
import { settingsTaskOverview } from "./setup";

export async function modelsView(ctx: Context): Promise<HTMLElement> {
  return h("div", { class: "models-settings" }, settingsTaskOverview(ctx, true), ...await inferenceModelsView(ctx));
}
