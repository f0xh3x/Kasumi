/** Power actions the launcher can show as buttons ([widgets.launcher] power), in
 *  display order: id (a run_action action) -> [label, glyph, needs a second click]. */
export const POWER_ACTIONS: Record<string, [label: string, glyph: number, confirm: boolean]> = {
  lock: ["Lock", 0xe72e, false],
  sleep: ["Sleep", 0xe708, false],
  hibernate: ["Hibernate", 0xe823, false],
  sign_out: ["Sign out", 0xe8f8, true],
  restart: ["Restart", 0xe777, true],
  shutdown: ["Shut down", 0xe7e8, true],
};
