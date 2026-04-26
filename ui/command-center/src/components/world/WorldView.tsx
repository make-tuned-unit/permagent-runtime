import { useEffect, useRef } from 'react';

export function WorldView({ visible = true }: { visible?: boolean }) {
  const canvasRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Pause animations when not visible
    if (!visible && canvasRef.current) {
      canvasRef.current.style.animationPlayState = 'paused';
    } else if (canvasRef.current) {
      canvasRef.current.style.animationPlayState = 'running';
    }
  }, [visible]);

  return (
    <div ref={canvasRef} className="flex h-full w-full items-center justify-center bg-[#0A0E17]">
      <div className="text-center">
        <div className="text-4xl mb-4">🌍</div>
        <h2 className="text-lg font-semibold text-dark-text">World View</h2>
        <p className="text-sm text-dark-muted mt-2">Global agent activity and connections</p>
      </div>
    </div>
  );
}
