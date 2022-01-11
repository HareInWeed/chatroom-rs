import React, { FC } from "react";

import { AUTHOR } from "about";

import {
  Avatar,
  Button,
  TextField,
  Link,
  Box,
  Typography,
  Container,
  CircularProgress,
} from "@mui/material";
import { invoke } from "@tauri-apps/api/tauri";

import { useForm, Controller } from "react-hook-form";

import { useSnackbar } from "notistack";

type ChangePassData = {
  old_password: string;
  new_password: string;
};

interface ChangePassModalProps {
  onClose: () => void;
}

const ChangePassModal: FC<ChangePassModalProps> = ({ onClose }) => {
  const { enqueueSnackbar } = useSnackbar();
  const {
    handleSubmit,
    setError,
    control,
    formState: { isSubmitting },
  } = useForm<ChangePassData>();

  return (
    <Box
      component="form"
      sx={{
        position: "absolute" as "absolute",
        top: "50%",
        left: "50%",
        transform: "translate(-50%, -50%)",
        width: 400,
        bgcolor: "background.paper",
        border: "1px solid #ccc",
        boxShadow: 24,
        borderRadius: 1,
        p: 4,

        display: "flex",
        flexDirection: "column",
      }}
      onSubmit={handleSubmit(async (data) => {
        try {
          await invoke("change_password", {
            old: data.old_password,
            new: data.new_password,
          });
          enqueueSnackbar("密码修改成功", { variant: "success" });
          onClose();
        } catch (err) {
          const msg = (err as any).msg;
          if (typeof msg === "string") {
            if (msg === "username or password are invalid") {
              setError("old_password", { message: "密码不正确" });
            } else {
              console.log(msg);
            }
          } else {
            console.error(err);
          }
        }
      })}
      noValidate
    >
      <Box sx={{ display: "flex", flexDirection: "column" }}>
        <Typography component="h1" variant="h6" sx={{ ml: 1 }}>
          修改密码
        </Typography>
        <Controller
          name="old_password"
          control={control}
          defaultValue=""
          rules={{ required: "请输入旧密码" }}
          render={({
            field: { onChange, onBlur, value, ref },
            fieldState: { error },
            formState: { isSubmitting },
          }) => (
            <TextField
              id="password"
              margin="normal"
              autoFocus
              required
              fullWidth
              label="旧密码"
              type="password"
              error={!!error}
              helperText={error && error.message}
              disabled={isSubmitting}
              value={value}
              onChange={onChange}
              onBlur={onBlur}
              inputRef={ref}
            />
          )}
        />
        <Controller
          name="new_password"
          control={control}
          defaultValue=""
          rules={{ required: "请输入新密码" }}
          render={({
            field: { onChange, onBlur, value, ref },
            fieldState: { error },
            formState: { isSubmitting },
          }) => (
            <TextField
              id="password"
              margin="normal"
              required
              fullWidth
              label="新密码"
              type="password"
              autoComplete="current-password"
              error={!!error}
              helperText={error && error.message}
              disabled={isSubmitting}
              value={value}
              onChange={onChange}
              onBlur={onBlur}
              inputRef={ref}
            />
          )}
        />
        <Box sx={{ display: "flex", flexDirection: "row", mt: 2 }}>
          <Button type="submit" disabled={isSubmitting} sx={{ ml: 2 }}>
            确认
          </Button>
          <Button
            disabled={isSubmitting}
            onClick={(event) => {
              event.preventDefault();
              onClose();
            }}
            sx={{ mr: 2, ml: "auto", color: "error.main" }}
          >
            取消
          </Button>
        </Box>
      </Box>
    </Box>
  );
};

export default ChangePassModal;
