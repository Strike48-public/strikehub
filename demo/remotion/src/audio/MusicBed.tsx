import { Audio, staticFile, interpolate, useCurrentFrame } from "remotion";

/**
 * MusicBed component — background music with volume ducking during voiceover.
 *
 * Usage:
 *   <MusicBed
 *     src="audio/music.mp3"
 *     baseVolume={0.15}
 *     duckVolume={0.05}
 *     duckRanges={[
 *       { from: 0, to: 108 },      // intro VO
 *       { from: 180, to: 393 },    // scan VO
 *       // ... etc
 *     ]}
 *   />
 *
 * The music plays continuously; volume ducks down during specified frame ranges.
 */

export interface DuckRange {
  from: number; // frame
  to: number;   // frame
}

interface MusicBedProps {
  src: string; // relative to public/, e.g. "audio/music.mp3"
  baseVolume?: number; // normal volume (default 0.15)
  duckVolume?: number; // ducked volume during VO (default 0.05)
  duckRanges: DuckRange[];
  totalDurationInFrames: number;
}

export const MusicBed: React.FC<MusicBedProps> = ({
  src,
  baseVolume = 0.15,
  duckVolume = 0.05,
  duckRanges,
  totalDurationInFrames,
}) => {
  const frame = useCurrentFrame();

  // Determine if current frame is in a duck range
  const isDucked = duckRanges.some((r) => frame >= r.from && frame < r.to);

  // Smooth fade in/out at duck boundaries (5 frame fade)
  let volume = baseVolume;
  for (const range of duckRanges) {
    if (frame >= range.from - 5 && frame < range.from) {
      // Fading down
      volume = interpolate(frame, [range.from - 5, range.from], [baseVolume, duckVolume], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      });
    } else if (frame >= range.to && frame < range.to + 5) {
      // Fading up
      volume = interpolate(frame, [range.to, range.to + 5], [duckVolume, baseVolume], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      });
    } else if (isDucked) {
      volume = duckVolume;
    }
  }

  return <Audio src={staticFile(src)} volume={volume} />;
};
