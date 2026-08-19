import { Composition, registerRoot } from "remotion";
import { Reel, reelDuration } from "./Reel.tsx";

export const RemotionRoot: React.FC = () => (
  <Composition
    id="Reel"
    component={Reel}
    durationInFrames={reelDuration()}
    fps={30}
    width={1920}
    height={1080}
  />
);

registerRoot(RemotionRoot);
