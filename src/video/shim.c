/* The Displayer's and the MP4 render's FFmpeg calls (`src/video/mod.rs`), behind a small C surface so that the Rust
 * side needs no bindings to FFmpeg's structs: a decoder of one stream of a media file (frames as RGBA at a size the
 * caller sets, sound as interleaved stereo float at the output's rate), and an MP4 encoder (H.264 from RGBA frames,
 * AAC from planar stereo float).
 *
 * A decoder reads a path, or on Android a file descriptor (a picked document's) through its own reads at its own
 * offset, so that several decoders share one descriptor. Hardware decoding and encoding go through MediaCodec where
 * FFmpeg was built with it (Android), falling back to FFmpeg's own decoders and x264.
 *
 * Part of Honeydew, GPL-3.0-or-later. */

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/display.h>
#include <libavutil/imgutils.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

#ifndef _WIN32
#include <sys/stat.h>
#include <unistd.h>
#endif

/* Bilinear scaling with exact rounding: swscale's plain C path (no assembly, as the host and armv7 builds are) gets the
 * colours wrong without SWS_ACCURATE_RND. */

static void put_error(char *err, int len, const char *what, int code) {
    if (!err || len <= 0)
        return;
    if (code) {
        char text[128];
        av_strerror(code, text, sizeof(text));
        snprintf(err, (size_t)len, "%s: %s", what, text);
    } else {
        snprintf(err, (size_t)len, "%s", what);
    }
}

/* ---- Reading a file descriptor ---------------------------------------------------------------------------------- */

typedef struct {
    int fd;
    int64_t pos;
    int64_t size;
} FdIo;

#ifndef _WIN32
static int fd_read(void *opaque, uint8_t *buf, int size) {
    FdIo *io = opaque;
    ssize_t n;
    do {
        n = pread(io->fd, buf, (size_t)size, (off_t)io->pos);
    } while (n < 0 && errno == EINTR);
    if (n < 0)
        return AVERROR(errno);
    if (n == 0)
        return AVERROR_EOF;
    io->pos += n;
    return (int)n;
}

static int64_t fd_seek(void *opaque, int64_t offset, int whence) {
    FdIo *io = opaque;
    if (whence & AVSEEK_SIZE)
        return io->size >= 0 ? io->size : AVERROR(ENOSYS);
    whence &= ~AVSEEK_FORCE;
    switch (whence) {
    case SEEK_SET:
        io->pos = offset;
        break;
    case SEEK_CUR:
        io->pos += offset;
        break;
    case SEEK_END:
        if (io->size < 0)
            return AVERROR(ENOSYS);
        io->pos = io->size + offset;
        break;
    default:
        return AVERROR(EINVAL);
    }
    return io->pos;
}
#endif

/* ---- The decoder ------------------------------------------------------------------------------------------------- */

typedef struct HdDecoder {
    AVFormatContext *fmt;
    AVIOContext *avio;
    FdIo *io;
    AVCodecContext *dec;
    int stream;
    int audio;
    int hardware;
    AVPacket *pkt;
    AVFrame *frame;
    int draining;
    int finished;
    double time_base;
    double start;
    /* Video: the display rotation (0, 90, 180, 270, clockwise), the output size, the frame as RGBA. */
    int rotation;
    int out_w, out_h;
    struct SwsContext *sws;
    uint8_t *scaled;
    uint8_t *rgba;
    int rgba_cap;
    /* Audio: the output rate and the converted frames, interleaved stereo. */
    int out_rate;
    SwrContext *swr;
    /* The input the resampler was set up for: a stream whose format, rate or channels change midway (an HE-AAC
     * stream's first frames) sets it up again, rather than reading planes that are not there. */
    int in_format, in_rate, in_channels;
    float *pcm;
    int pcm_cap;
} HdDecoder;

static const char *hardware_name(enum AVCodecID id) {
    switch (id) {
    case AV_CODEC_ID_H264:
        return "h264_mediacodec";
    case AV_CODEC_ID_HEVC:
        return "hevc_mediacodec";
    case AV_CODEC_ID_VP8:
        return "vp8_mediacodec";
    case AV_CODEC_ID_VP9:
        return "vp9_mediacodec";
    case AV_CODEC_ID_AV1:
        return "av1_mediacodec";
    default:
        return NULL;
    }
}

static int open_codec(HdDecoder *d, const AVCodec *codec, AVStream *st) {
    AVCodecContext *ctx = avcodec_alloc_context3(codec);
    if (!ctx)
        return AVERROR(ENOMEM);
    int ret = avcodec_parameters_to_context(ctx, st->codecpar);
    if (ret < 0) {
        avcodec_free_context(&ctx);
        return ret;
    }
    ctx->pkt_timebase = st->time_base;
    if (!d->audio && !(codec->capabilities & AV_CODEC_CAP_HARDWARE)) {
        ctx->thread_count = 0;
        ctx->thread_type = FF_THREAD_FRAME | FF_THREAD_SLICE;
    }
    ret = avcodec_open2(ctx, codec, NULL);
    if (ret < 0) {
        avcodec_free_context(&ctx);
        return ret;
    }
    d->dec = ctx;
    return 0;
}

void hd_dec_close(HdDecoder *d) {
    if (!d)
        return;
    avcodec_free_context(&d->dec);
    if (d->fmt)
        avformat_close_input(&d->fmt);
    if (d->avio) {
        av_freep(&d->avio->buffer);
        avio_context_free(&d->avio);
    }
    av_free(d->io);
    av_packet_free(&d->pkt);
    av_frame_free(&d->frame);
    sws_freeContext(d->sws);
    swr_free(&d->swr);
    av_free(d->scaled);
    av_free(d->rgba);
    av_free(d->pcm);
    av_free(d);
}

/* Opens the best video (`audio` 0) or sound (`audio` 1) stream of the file at `path`, or of the descriptor `fd` when
 * it is ≥ 0. Video tries MediaCodec first with `hardware`. Sound comes out at `out_rate` Hz. NULL, with the reason in
 * `err`, on failure (or when the file has no such stream). */
HdDecoder *hd_dec_open(const char *path, int fd, int audio, int hardware, int out_rate, char *err, int err_len) {
    HdDecoder *d = av_mallocz(sizeof(HdDecoder));
    if (!d) {
        put_error(err, err_len, "Out of memory", 0);
        return NULL;
    }
    d->audio = audio;
    d->out_rate = out_rate > 0 ? out_rate : 48000;
    d->stream = -1;
    int ret;
    d->fmt = avformat_alloc_context();
    if (!d->fmt) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    if (fd >= 0) {
#ifdef _WIN32
        put_error(err, err_len, "File descriptors are not supported here", 0);
        goto fail;
#else
        d->io = av_mallocz(sizeof(FdIo));
        unsigned char *buffer = av_malloc(65536);
        if (!d->io || !buffer) {
            av_free(buffer);
            put_error(err, err_len, "Out of memory", 0);
            goto fail;
        }
        struct stat info;
        d->io->fd = fd;
        d->io->size = fstat(fd, &info) == 0 && S_ISREG(info.st_mode) ? (int64_t)info.st_size : -1;
        d->avio = avio_alloc_context(buffer, 65536, 0, d->io, fd_read, NULL, fd_seek);
        if (!d->avio) {
            av_free(buffer);
            put_error(err, err_len, "Out of memory", 0);
            goto fail;
        }
        d->avio->seekable = d->io->size >= 0 ? AVIO_SEEKABLE_NORMAL : 0;
        d->fmt->pb = d->avio;
        d->fmt->flags |= AVFMT_FLAG_CUSTOM_IO;
#endif
    }
    ret = avformat_open_input(&d->fmt, fd >= 0 ? NULL : path, NULL, NULL);
    if (ret < 0) {
        /* avformat_open_input frees the context on failure. */
        d->fmt = NULL;
        put_error(err, err_len, "Cannot open the file", ret);
        goto fail;
    }
    ret = avformat_find_stream_info(d->fmt, NULL);
    if (ret < 0) {
        put_error(err, err_len, "Cannot read the file", ret);
        goto fail;
    }
    const AVCodec *codec = NULL;
    d->stream = av_find_best_stream(d->fmt, audio ? AVMEDIA_TYPE_AUDIO : AVMEDIA_TYPE_VIDEO, -1, -1, &codec, 0);
    if (d->stream < 0) {
        put_error(err, err_len, audio ? "No sound in the file" : "No video in the file", 0);
        goto fail;
    }
    for (unsigned i = 0; i < d->fmt->nb_streams; i++)
        d->fmt->streams[i]->discard = (int)i == d->stream ? AVDISCARD_DEFAULT : AVDISCARD_ALL;
    AVStream *st = d->fmt->streams[d->stream];
    d->time_base = av_q2d(st->time_base);
    d->start = st->start_time != AV_NOPTS_VALUE ? (double)st->start_time * d->time_base : 0.0;
    ret = AVERROR_DECODER_NOT_FOUND;
    if (!audio && hardware) {
        const char *name = hardware_name(st->codecpar->codec_id);
        const AVCodec *hw = name ? avcodec_find_decoder_by_name(name) : NULL;
        if (hw && open_codec(d, hw, st) >= 0) {
            d->hardware = 1;
            ret = 0;
        }
    }
    if (!d->dec) {
        const AVCodec *sw = avcodec_find_decoder(st->codecpar->codec_id);
        ret = sw ? open_codec(d, sw, st) : AVERROR_DECODER_NOT_FOUND;
        if (ret < 0) {
            /* AV1 has no software decoder in this build: only MediaCodec's. */
            put_error(err, err_len, "This format cannot be played here", ret);
            goto fail;
        }
    }
    if (!audio) {
        const AVPacketSideData *side = av_packet_side_data_get(
            st->codecpar->coded_side_data, st->codecpar->nb_coded_side_data, AV_PKT_DATA_DISPLAYMATRIX);
        if (side && side->size >= 9 * sizeof(int32_t)) {
            double angle = -av_display_rotation_get((const int32_t *)side->data);
            int r = (int)((angle < 0 ? angle + 360.0 : angle) + 45.0) / 90 % 4;
            d->rotation = r * 90;
        }
    }
    d->pkt = av_packet_alloc();
    d->frame = av_frame_alloc();
    if (!d->pkt || !d->frame) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    return d;
fail:
    hd_dec_close(d);
    return NULL;
}

/* The stream's length in seconds (0 if unknown), its display size (rotation and pixel aspect applied), whether the
 * decoder is MediaCodec's, and for sound its source rate. */
void hd_dec_info(HdDecoder *d, double *duration, int *width, int *height, int *hardware) {
    AVStream *st = d->fmt->streams[d->stream];
    double secs = 0.0;
    if (d->fmt->duration != AV_NOPTS_VALUE && d->fmt->duration > 0)
        secs = (double)d->fmt->duration / AV_TIME_BASE;
    else if (st->duration != AV_NOPTS_VALUE && st->duration > 0)
        secs = (double)st->duration * d->time_base;
    *duration = secs;
    int w = st->codecpar->width, h = st->codecpar->height;
    AVRational sar = st->sample_aspect_ratio.num ? st->sample_aspect_ratio : st->codecpar->sample_aspect_ratio;
    if (sar.num > 0 && sar.den > 0 && sar.num != sar.den)
        w = (int)((double)w * sar.num / sar.den + 0.5);
    if (d->rotation == 90 || d->rotation == 270) {
        int t = w;
        w = h;
        h = t;
    }
    *width = w;
    *height = h;
    *hardware = d->hardware;
}

/* The video frames' size from now on, as shown (after the rotation). */
void hd_dec_set_size(HdDecoder *d, int width, int height) {
    d->out_w = width > 0 ? width : 1;
    d->out_h = height > 0 ? height : 1;
}

/* Goes to the key frame at or before `secs`, so that the next frames lead up to it. */
int hd_dec_seek(HdDecoder *d, double secs) {
    AVStream *st = d->fmt->streams[d->stream];
    int64_t ts = (int64_t)((secs + d->start) / d->time_base);
    if (st->start_time != AV_NOPTS_VALUE && ts < st->start_time)
        ts = st->start_time;
    int ret = avformat_seek_file(d->fmt, d->stream, INT64_MIN, ts, ts, 0);
    if (ret < 0)
        ret = av_seek_frame(d->fmt, d->stream, ts, AVSEEK_FLAG_BACKWARD);
    avcodec_flush_buffers(d->dec);
    if (d->swr) {
        swr_close(d->swr);
        swr_init(d->swr);
    }
    d->draining = 0;
    d->finished = 0;
    return ret < 0 ? ret : 0;
}

/* The next decoded frame into d->frame: 1, or 0 at the end, < 0 on an error. */
static int next_frame(HdDecoder *d) {
    for (;;) {
        if (d->finished)
            return 0;
        int ret = avcodec_receive_frame(d->dec, d->frame);
        if (ret == 0)
            return 1;
        if (ret == AVERROR_EOF) {
            d->finished = 1;
            return 0;
        }
        if (ret != AVERROR(EAGAIN))
            return ret;
        if (d->draining) {
            d->finished = 1;
            return 0;
        }
        ret = av_read_frame(d->fmt, d->pkt);
        if (ret < 0) {
            d->draining = 1;
            avcodec_send_packet(d->dec, NULL);
            continue;
        }
        if (d->pkt->stream_index != d->stream) {
            av_packet_unref(d->pkt);
            continue;
        }
        ret = avcodec_send_packet(d->dec, d->pkt);
        av_packet_unref(d->pkt);
        if (ret < 0 && ret != AVERROR(EAGAIN) && ret != AVERROR_INVALIDDATA)
            return ret;
    }
}

static double frame_secs(HdDecoder *d) {
    int64_t pts = d->frame->best_effort_timestamp;
    if (pts == AV_NOPTS_VALUE)
        pts = d->frame->pts;
    if (pts == AV_NOPTS_VALUE)
        return -1.0;
    return (double)pts * d->time_base - d->start;
}

/* The next video frame, as RGBA at the set size: 1 with `*rgba` (out_w × out_h × 4, rows packed), its size and its
 * time in seconds from the stream's start; 0 at the end; < 0 on an error. With `skip` the frame is only decoded, not
 * converted (`*rgba` is NULL). */
int hd_dec_video(HdDecoder *d, int skip, const uint8_t **rgba, int *width, int *height, double *secs) {
    *rgba = NULL;
    int ret = next_frame(d);
    if (ret <= 0)
        return ret;
    *secs = frame_secs(d);
    if (skip) {
        av_frame_unref(d->frame);
        return 1;
    }
    if (d->out_w <= 0 || d->out_h <= 0) {
        int w, h, hw;
        double dur;
        hd_dec_info(d, &dur, &w, &h, &hw);
        hd_dec_set_size(d, w, h);
    }
    int turned = d->rotation == 90 || d->rotation == 270;
    int sw = turned ? d->out_h : d->out_w, sh = turned ? d->out_w : d->out_h;
    d->sws = sws_getCachedContext(d->sws, d->frame->width, d->frame->height, (enum AVPixelFormat)d->frame->format, sw,
                                  sh, AV_PIX_FMT_RGBA, SWS_BILINEAR | SWS_ACCURATE_RND, NULL, NULL, NULL);
    if (!d->sws) {
        av_frame_unref(d->frame);
        return AVERROR(EINVAL);
    }
    int need = d->out_w * d->out_h * 4;
    if (need > d->rgba_cap) {
        av_free(d->rgba);
        av_free(d->scaled);
        d->rgba = av_malloc((size_t)need);
        d->scaled = av_malloc((size_t)need);
        d->rgba_cap = d->rgba && d->scaled ? need : 0;
        if (!d->rgba_cap) {
            av_frame_unref(d->frame);
            return AVERROR(ENOMEM);
        }
    }
    uint8_t *target = d->rotation ? d->scaled : d->rgba;
    uint8_t *planes[4] = {target, NULL, NULL, NULL};
    int strides[4] = {sw * 4, 0, 0, 0};
    sws_scale(d->sws, (const uint8_t *const *)d->frame->data, d->frame->linesize, 0, d->frame->height, planes,
              strides);
    av_frame_unref(d->frame);
    if (d->rotation) {
        const uint32_t *src = (const uint32_t *)d->scaled;
        uint32_t *dst = (uint32_t *)d->rgba;
        int ow = d->out_w, oh = d->out_h;
        for (int y = 0; y < sh; y++) {
            for (int x = 0; x < sw; x++) {
                uint32_t px = src[y * sw + x];
                int tx, ty;
                if (d->rotation == 90) {
                    tx = sh - 1 - y;
                    ty = x;
                } else if (d->rotation == 180) {
                    tx = sw - 1 - x;
                    ty = sh - 1 - y;
                } else {
                    tx = y;
                    ty = sw - 1 - x;
                }
                dst[ty * ow + tx] = px;
            }
        }
        (void)oh;
    }
    *rgba = d->rgba;
    *width = d->out_w;
    *height = d->out_h;
    return 1;
}

/* The next sound, as interleaved stereo float at the output rate: the frame count (≥ 0; 0 can be a frame the
 * resampler holds back) with `*pcm` and the time of its first frame in seconds from the stream's start; -1 at the end
 * (AVERROR_EOF); < 0 on an error. */
int hd_dec_audio(HdDecoder *d, const float **pcm, double *secs) {
    *pcm = NULL;
    int ret = next_frame(d);
    if (ret == 0)
        return AVERROR_EOF;
    if (ret < 0)
        return ret;
    AVFrame *f = d->frame;
    *secs = frame_secs(d);
    if (f->nb_samples <= 0 || f->sample_rate <= 0) {
        av_frame_unref(f);
        return 0;
    }
    if (d->swr && (f->format != d->in_format || f->sample_rate != d->in_rate ||
                   f->ch_layout.nb_channels != d->in_channels))
        swr_free(&d->swr);
    if (!d->swr) {
        d->in_format = f->format;
        d->in_rate = f->sample_rate;
        d->in_channels = f->ch_layout.nb_channels;
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        AVChannelLayout in;
        if (f->ch_layout.nb_channels > 0)
            av_channel_layout_copy(&in, &f->ch_layout);
        else
            av_channel_layout_default(&in, 2);
        ret = swr_alloc_set_opts2(&d->swr, &stereo, AV_SAMPLE_FMT_FLT, d->out_rate, &in, (enum AVSampleFormat)f->format,
                                  f->sample_rate, 0, NULL);
        av_channel_layout_uninit(&in);
        if (ret < 0 || swr_init(d->swr) < 0) {
            swr_free(&d->swr);
            av_frame_unref(f);
            return ret < 0 ? ret : AVERROR(EINVAL);
        }
    }
    int cap = swr_get_out_samples(d->swr, f->nb_samples) + 32;
    if (cap * 2 > d->pcm_cap) {
        av_free(d->pcm);
        d->pcm = av_malloc((size_t)cap * 2 * sizeof(float));
        d->pcm_cap = d->pcm ? cap * 2 : 0;
        if (!d->pcm) {
            av_frame_unref(f);
            return AVERROR(ENOMEM);
        }
    }
    uint8_t *out[1] = {(uint8_t *)d->pcm};
    int n = swr_convert(d->swr, out, cap, (const uint8_t **)f->extended_data, f->nb_samples);
    av_frame_unref(f);
    if (n < 0)
        return n;
    *pcm = d->pcm;
    return n;
}

/* ---- The MP4 encoder --------------------------------------------------------------------------------------------- */

typedef struct HdEncoder {
    AVFormatContext *fmt;
    AVCodecContext *venc;
    AVCodecContext *aenc;
    AVStream *vst;
    AVStream *ast;
    AVFrame *vframe;
    AVFrame *aframe;
    AVPacket *pkt;
    struct SwsContext *sws;
    int width, height;
    int64_t apts;
    float *left, *right;
    int queued, cap;
    int hardware;
    int header;
} HdEncoder;

void hd_enc_close(HdEncoder *e) {
    if (!e)
        return;
    avcodec_free_context(&e->venc);
    avcodec_free_context(&e->aenc);
    av_frame_free(&e->vframe);
    av_frame_free(&e->aframe);
    av_packet_free(&e->pkt);
    sws_freeContext(e->sws);
    if (e->fmt) {
        if (e->fmt->pb)
            avio_closep(&e->fmt->pb);
        avformat_free_context(e->fmt);
    }
    av_free(e->left);
    av_free(e->right);
    av_free(e);
}

static AVCodecContext *video_encoder(const AVCodec *codec, int width, int height, int fps, int bitrate,
                                     enum AVPixelFormat pix, const char *preset, int global) {
    AVCodecContext *c = avcodec_alloc_context3(codec);
    if (!c)
        return NULL;
    c->width = width;
    c->height = height;
    c->time_base = (AVRational){1, fps};
    c->framerate = (AVRational){fps, 1};
    c->pix_fmt = pix;
    c->bit_rate = bitrate;
    c->gop_size = fps * 2;
    c->max_b_frames = 0;
    c->color_range = AVCOL_RANGE_MPEG;
    c->colorspace = AVCOL_SPC_BT709;
    c->color_primaries = AVCOL_PRI_BT709;
    c->color_trc = AVCOL_TRC_BT709;
    if (global)
        c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    if (preset) {
        av_opt_set(c->priv_data, "preset", preset, 0);
        av_opt_set(c->priv_data, "profile", "high", 0);
    } else {
        av_opt_set_int(c->priv_data, "ndk_codec", 1, 0);
        av_opt_set(c->priv_data, "bitrate_mode", "vbr", 0);
    }
    if (avcodec_open2(c, codec, NULL) < 0) {
        avcodec_free_context(&c);
        return NULL;
    }
    return c;
}

/* An MP4 file at `path`: H.264 `width` × `height` at `fps` and `bitrate` bit/s (MediaCodec's encoder first with
 * `hardware`, else x264 at `preset`), AAC stereo at `rate` Hz. NULL, with the reason in `err`, on failure. */
HdEncoder *hd_enc_open(const char *path, int width, int height, int fps, int rate, int bitrate, int hardware,
                       const char *preset, char *err, int err_len) {
    HdEncoder *e = av_mallocz(sizeof(HdEncoder));
    if (!e) {
        put_error(err, err_len, "Out of memory", 0);
        return NULL;
    }
    e->width = width;
    e->height = height;
    int ret = avformat_alloc_output_context2(&e->fmt, NULL, "mp4", path);
    if (ret < 0 || !e->fmt) {
        put_error(err, err_len, "Cannot make an MP4 file", ret);
        goto fail;
    }
    int global = (e->fmt->oformat->flags & AVFMT_GLOBALHEADER) != 0;
    if (hardware) {
        const AVCodec *hw = avcodec_find_encoder_by_name("h264_mediacodec");
        if (hw) {
            e->venc = video_encoder(hw, width, height, fps, bitrate, AV_PIX_FMT_NV12, NULL, global);
            if (!e->venc)
                e->venc = video_encoder(hw, width, height, fps, bitrate, AV_PIX_FMT_YUV420P, NULL, global);
            e->hardware = e->venc != NULL;
        }
    }
    if (!e->venc) {
        const AVCodec *x264 = avcodec_find_encoder_by_name("libx264");
        if (x264)
            e->venc = video_encoder(x264, width, height, fps, bitrate, AV_PIX_FMT_YUV420P,
                                    preset && *preset ? preset : "veryfast", global);
    }
    if (!e->venc) {
        put_error(err, err_len, "No H.264 encoder could start", 0);
        goto fail;
    }
    const AVCodec *aac = avcodec_find_encoder(AV_CODEC_ID_AAC);
    e->aenc = aac ? avcodec_alloc_context3(aac) : NULL;
    if (!e->aenc) {
        put_error(err, err_len, "No AAC encoder", 0);
        goto fail;
    }
    e->aenc->sample_fmt = AV_SAMPLE_FMT_FLTP;
    e->aenc->sample_rate = rate;
    e->aenc->bit_rate = 192000;
    e->aenc->time_base = (AVRational){1, rate};
    AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
    av_channel_layout_copy(&e->aenc->ch_layout, &stereo);
    if (global)
        e->aenc->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    ret = avcodec_open2(e->aenc, aac, NULL);
    if (ret < 0) {
        put_error(err, err_len, "The AAC encoder could not start", ret);
        goto fail;
    }
    e->vst = avformat_new_stream(e->fmt, NULL);
    e->ast = avformat_new_stream(e->fmt, NULL);
    if (!e->vst || !e->ast) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    avcodec_parameters_from_context(e->vst->codecpar, e->venc);
    avcodec_parameters_from_context(e->ast->codecpar, e->aenc);
    e->vst->time_base = e->venc->time_base;
    e->ast->time_base = e->aenc->time_base;
    e->vst->avg_frame_rate = e->venc->framerate;
    ret = avio_open(&e->fmt->pb, path, AVIO_FLAG_WRITE);
    if (ret < 0) {
        put_error(err, err_len, "Cannot write the file", ret);
        goto fail;
    }
    AVDictionary *opts = NULL;
    av_dict_set(&opts, "movflags", "+faststart", 0);
    ret = avformat_write_header(e->fmt, &opts);
    av_dict_free(&opts);
    if (ret < 0) {
        put_error(err, err_len, "Cannot write the MP4 header", ret);
        goto fail;
    }
    e->header = 1;
    e->pkt = av_packet_alloc();
    e->vframe = av_frame_alloc();
    e->aframe = av_frame_alloc();
    if (!e->pkt || !e->vframe || !e->aframe) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    e->vframe->format = e->venc->pix_fmt;
    e->vframe->width = width;
    e->vframe->height = height;
    if (av_frame_get_buffer(e->vframe, 0) < 0) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    int size = e->aenc->frame_size > 0 ? e->aenc->frame_size : 1024;
    e->aframe->format = AV_SAMPLE_FMT_FLTP;
    e->aframe->sample_rate = rate;
    e->aframe->nb_samples = size;
    av_channel_layout_copy(&e->aframe->ch_layout, &stereo);
    if (av_frame_get_buffer(e->aframe, 0) < 0) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    e->cap = size * 4 + 8192;
    e->left = av_malloc((size_t)e->cap * sizeof(float));
    e->right = av_malloc((size_t)e->cap * sizeof(float));
    if (!e->left || !e->right) {
        put_error(err, err_len, "Out of memory", 0);
        goto fail;
    }
    return e;
fail:
    hd_enc_close(e);
    return NULL;
}

int hd_enc_hardware(HdEncoder *e) {
    return e->hardware;
}

static int drain(HdEncoder *e, AVCodecContext *c, AVStream *st) {
    for (;;) {
        int ret = avcodec_receive_packet(c, e->pkt);
        if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
            return 0;
        if (ret < 0)
            return ret;
        av_packet_rescale_ts(e->pkt, c->time_base, st->time_base);
        e->pkt->stream_index = st->index;
        ret = av_interleaved_write_frame(e->fmt, e->pkt);
        if (ret < 0)
            return ret;
    }
}

/* Frame number `index` of the video, RGBA width × height × 4, rows packed. */
int hd_enc_video(HdEncoder *e, const uint8_t *rgba, int64_t index) {
    int ret = av_frame_make_writable(e->vframe);
    if (ret < 0)
        return ret;
    e->sws = sws_getCachedContext(e->sws, e->width, e->height, AV_PIX_FMT_RGBA, e->width, e->height,
                                  e->venc->pix_fmt, SWS_BILINEAR | SWS_ACCURATE_RND, NULL, NULL, NULL);
    if (!e->sws)
        return AVERROR(EINVAL);
    const uint8_t *planes[4] = {rgba, NULL, NULL, NULL};
    int strides[4] = {e->width * 4, 0, 0, 0};
    sws_scale(e->sws, planes, strides, 0, e->height, e->vframe->data, e->vframe->linesize);
    e->vframe->pts = index;
    ret = avcodec_send_frame(e->venc, e->vframe);
    if (ret < 0 && ret != AVERROR(EAGAIN))
        return ret;
    return drain(e, e->venc, e->vst);
}

static int send_audio(HdEncoder *e, int frames) {
    int ret = av_frame_make_writable(e->aframe);
    if (ret < 0)
        return ret;
    e->aframe->nb_samples = frames;
    memcpy(e->aframe->data[0], e->left, (size_t)frames * sizeof(float));
    memcpy(e->aframe->data[1], e->right, (size_t)frames * sizeof(float));
    e->aframe->pts = e->apts;
    e->apts += frames;
    ret = avcodec_send_frame(e->aenc, e->aframe);
    if (ret < 0)
        return ret;
    e->queued -= frames;
    memmove(e->left, e->left + frames, (size_t)e->queued * sizeof(float));
    memmove(e->right, e->right + frames, (size_t)e->queued * sizeof(float));
    return drain(e, e->aenc, e->ast);
}

/* `frames` frames of sound, the channels apart. */
int hd_enc_audio(HdEncoder *e, const float *left, const float *right, int frames) {
    int size = e->aenc->frame_size > 0 ? e->aenc->frame_size : 1024;
    while (frames > 0) {
        int n = e->cap - e->queued;
        if (n > frames)
            n = frames;
        memcpy(e->left + e->queued, left, (size_t)n * sizeof(float));
        memcpy(e->right + e->queued, right, (size_t)n * sizeof(float));
        e->queued += n;
        left += n;
        right += n;
        frames -= n;
        while (e->queued >= size) {
            int ret = send_audio(e, size);
            if (ret < 0)
                return ret;
        }
    }
    return 0;
}

/* Flushes both encoders and writes the file's end. */
int hd_enc_finish(HdEncoder *e) {
    int ret;
    if (e->queued > 0) {
        int size = e->aenc->frame_size > 0 ? e->aenc->frame_size : 1024;
        int last = e->queued;
        if (!(e->aenc->codec->capabilities & AV_CODEC_CAP_SMALL_LAST_FRAME)) {
            memset(e->left + last, 0, (size_t)(size - last) * sizeof(float));
            memset(e->right + last, 0, (size_t)(size - last) * sizeof(float));
            e->queued = size;
            last = size;
        }
        ret = send_audio(e, last);
        if (ret < 0)
            return ret;
    }
    avcodec_send_frame(e->venc, NULL);
    ret = drain(e, e->venc, e->vst);
    if (ret < 0)
        return ret;
    avcodec_send_frame(e->aenc, NULL);
    ret = drain(e, e->aenc, e->ast);
    if (ret < 0)
        return ret;
    ret = av_write_trailer(e->fmt);
    if (ret < 0)
        return ret;
    return avio_closep(&e->fmt->pb);
}

void hd_error_text(int code, char *err, int err_len) {
    put_error(err, err_len, "FFmpeg", code);
}

/* FFmpeg's log down to its errors (it writes to stderr, which nobody reads on a phone). */
void hd_init(void) {
    av_log_set_level(AV_LOG_ERROR);
}
