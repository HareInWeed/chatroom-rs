import { useState, useCallback } from "react";
import { createContainer } from "unstated-next";

const usePath = (initialPath: string[] = []) => {
  const [path, set_path] = useState<string[]>(initialPath);

  const go_back = useCallback(() => {
    set_path((path) => {
      path.pop();
      return [...path];
    });
  }, [set_path]);

  const go_forward = useCallback(
    (p) => {
      set_path((path) => [...path, p]);
    },
    [set_path]
  );

  return { path, set_path, go_back, go_forward };
};

const PathState = createContainer(usePath);

export default PathState;
