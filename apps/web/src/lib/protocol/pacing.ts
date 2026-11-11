/** One timing contract shared by the server hold and the visible choreography. */
export const TABLE_MOTION = Object.freeze({ play: 700, read: 1500, collect: 780, stagger: 45 });
export const TRICK_PRESENTATION_MS =
	TABLE_MOTION.play + TABLE_MOTION.read + TABLE_MOTION.collect + 3 * TABLE_MOTION.stagger + 185;
