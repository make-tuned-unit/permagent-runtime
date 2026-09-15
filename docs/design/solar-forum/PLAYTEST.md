# Human-scale review route

Open `/ui/forum.html`. The architecture is an active study inside World, with no
separate product logo. Browser rendering/manual input and Unreal play-in-editor
cannot be verified with the unavailable UI automation in this session.

1. Choose **Commons → Walk the world**. Use WASD; Shift moves faster. Mouse capture
   is used when supported; otherwise drag the scene or use left/right arrows to look.
2. Walk across the southern bridge to **Arrival gardens**, then turn onto the
   planted promenade. The outer loop connects the Reading grove, Maker court and
   Council garden. These are destinations to explore, not fake running jobs.
3. Return to the forum. Approach the side staircase over its level landing and climb
   to the archive gallery. Use **E** within three meters of an agent to inspect it.
4. Press Escape or **Return to overview**. Place controls offer a quick route back
   to any district; choosing a place also offers walking from that area.
5. Change macOS appearance from light to dark and back. The scene should switch
   between neutral daylight and dark blue night lighting without reloading. It does
   not infer daytime from a clock or change your app-wide theme preference.
6. Compare the twelve agents through the selector: distinct portraits, silhouettes,
   working manners and example questions. Their activity colors retain the existing
   World semantics. Preparing a brief never starts a job.
7. Try a query only when you want to send it to the existing orchestrator conversation.
   Ask is a live send action; export remains a draft. The conversation retains the
   canonical message, approval and stop controls.

For Unreal, open `unreal/SolarForum/SolarForum.uproject` in the installed engine.
The generated map is `/Game/SolarForum/Maps/Forum`. The Blueprint/input assets are
compiled and the import uses real static triangle collision. Play behavior, visual
quality, performance, navigation and agent interactions still need a native review.
Duplicate the generated map before hand-editing; the importer updates its actors.
