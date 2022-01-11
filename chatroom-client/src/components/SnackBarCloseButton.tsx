import { IconButton } from "@mui/material";
import { Close as IconClose } from "@mui/icons-material";
import { useSnackbar, SnackbarKey } from "notistack";
import React, { FC } from "react";

export interface SnackbarCloseButtonProps {
  snackbarKey: SnackbarKey;
}

const SnackbarCloseButton: FC<SnackbarCloseButtonProps> = ({ snackbarKey }) => {
  const { closeSnackbar } = useSnackbar();

  return (
    <IconButton
      onClick={() => closeSnackbar(snackbarKey)}
      sx={{ color: "white" }}
    >
      <IconClose />
    </IconButton>
  );
};

export default SnackbarCloseButton;
