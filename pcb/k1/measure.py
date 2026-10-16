#!/usr/bin/env python3
"""Measure a part's pin holes on graph paper from a phone photo.

The user stabs the part's pins through the paper and photographs it backlit.
This fits the grid lines (1 mm squares) to get a pixel -> mm map that also
corrects the camera's perspective, then finds the holes:

    measure.py PHOTO LEFT,TOP,RIGHT,BOTTOM [--black] [--red ANNOTATED]

LEFT,TOP,RIGHT,BOTTOM: the pixel box around the holes.
--black: holes are the darkest pixels (backlit). Without it, marks are
  shadows darker than the paper around them (front-lit).
--red ANNOTATED: the same photo with a red bar drawn over each real hole
  (the user's; a person picks real holes out of torn paper better than a
  threshold does). Prints each bar's centre, length and angle.

Coordinates are grid squares from the box's first grid lines, x right, y
down, as seen in the photo: the side of the paper facing the camera.
"""

import sys

import numpy as np
from PIL import Image
from scipy import ndimage, signal

SQUARE_PX = 34  # rough grid spacing; only used to match lines between bands
BANDS = 8  # each grid line is located in this many strips, then fitted straight


def main():
    photo = sys.argv[1]
    left, top, right, bottom = (int(value) for value in sys.argv[2].split(","))
    image = np.asarray(Image.open(photo).convert("RGB"), dtype=float)[top:bottom, left:right]
    # Grid ink is cyan: green and blue well above red. Paper and holes aren't
    cyan = (image[..., 1] + image[..., 2]) / 2 - image[..., 0]

    to_grid = fit_grid(cyan)

    if "--red" in sys.argv:
        annotated = sys.argv[sys.argv.index("--red") + 1]
        marked = np.asarray(Image.open(annotated).convert("RGB"), dtype=float)[top:bottom, left:right]
        print_bars(marked, to_grid)
    else:
        print_holes(image, cyan, to_grid, left, top, black="--black" in sys.argv)


def fit_grid(cyan):
    """A homography from box pixels to grid squares, fitted to every grid
    line crossing"""
    height, width = cyan.shape
    vertical = fit_lines(locate_lines(cyan, across=False))
    horizontal = fit_lines(locate_lines(cyan, across=True))
    vertical, vertical_index = number_lines(vertical, height / 2)
    horizontal, horizontal_index = number_lines(horizontal, width / 2)

    pixels, squares = [], []
    for (v_intercept, v_slope), column in zip(vertical, vertical_index):
        for (h_intercept, h_slope), row in zip(horizontal, horizontal_index):
            # Vertical line: x = v_intercept + v_slope * y; horizontal: y = h_intercept + h_slope * x
            y_px = (h_intercept + h_slope * v_intercept) / (1 - h_slope * v_slope)
            x_px = v_intercept + v_slope * y_px
            pixels.append((x_px, y_px))
            squares.append((column, row))
    pixels = np.array(pixels)
    squares = np.array(squares, dtype=float)

    # Direct linear transform: least squares over all crossings
    equations = []
    for (x_px, y_px), (x_sq, y_sq) in zip(pixels, squares):
        equations.append([x_px, y_px, 1, 0, 0, 0, -x_sq * x_px, -x_sq * y_px, -x_sq])
        equations.append([0, 0, 0, x_px, y_px, 1, -y_sq * x_px, -y_sq * y_px, -y_sq])
    homography = np.linalg.svd(np.array(equations))[2][-1].reshape(3, 3)

    def to_grid(points):
        points = np.column_stack([points, np.ones(len(points))]) @ homography.T
        return points[:, :2] / points[:, 2:]

    residual = to_grid(pixels) - squares
    print(f"grid fit: {np.sqrt((residual ** 2).mean()):.3f} squares rms over {len(squares)} crossings")
    return to_grid


def locate_lines(cyan, across):
    """Grid line positions in each band: (band centre, [line positions])"""
    length = cyan.shape[1] if across else cyan.shape[0]
    bands = []
    for band in range(BANDS):
        start, stop = band * length // BANDS, (band + 1) * length // BANDS
        profile = cyan[:, start:stop].mean(axis=1) if across else cyan[start:stop, :].mean(axis=0)
        smooth = ndimage.gaussian_filter1d(profile, 1.5)
        peaks, _ = signal.find_peaks(smooth, distance=22, prominence=1.0)
        bands.append(((start + stop) / 2, peaks))
    return bands


def fit_lines(bands):
    """Follow each line from the middle band through the others; fit it straight
    (position = intercept + slope * band position)"""
    lines = []
    for peak in bands[len(bands) // 2][1]:
        points = []
        for centre, peaks in bands:
            if len(peaks):
                nearest = peaks[np.argmin(abs(peaks - peak))]
                if abs(nearest - peak) < SQUARE_PX / 3:
                    points.append((centre, nearest))
        if len(points) >= 5:
            centres, positions = np.array(points).T
            slope, intercept = np.polyfit(centres, positions, 1)
            lines.append((intercept, slope))
    return lines


def number_lines(lines, middle):
    """Each line's grid number. Spacing shrinks across the photo (perspective),
    so number by a quadratic fit and drop lines that don't land on an integer:
    false peaks, or lines pulled sideways by a hole"""
    positions = np.array([intercept + slope * middle for intercept, slope in lines])
    numbers = np.round((positions - positions[0]) / np.median(np.diff(positions)))
    for _ in range(3):
        exact = np.polyval(np.polyfit(positions, numbers, 2), positions)
        keep = abs(exact - np.round(exact)) < 0.1
        numbers = np.round(exact)
    return [line for line, ok in zip(lines, keep) if ok], numbers[keep].astype(int)


def print_holes(image, cyan, to_grid, left, top, black):
    gray = image.mean(axis=2)
    if black:
        dark = gray < 60  # backlit holes are near black; grid ink stays above ~80
    else:
        dark = (ndimage.median_filter(gray, size=41) - gray > 35) & (cyan < 25)
    labels, count = ndimage.label(ndimage.binary_opening(dark, iterations=1))
    print("  photo px          grid (x, y)        area  size px")
    for index in range(1, count + 1):
        rows, columns = np.nonzero(labels == index)
        if len(columns) < 25:
            continue
        centre = to_grid(np.array([[columns.mean(), rows.mean()]]))[0]
        print(f"  ({columns.mean() + left:6.0f},{rows.mean() + top:6.0f})  ({centre[0]:7.3f}, {centre[1]:7.3f})  "
              f"{len(columns):5d}  {np.ptp(columns) + 1}x{np.ptp(rows) + 1}")


def print_bars(marked, to_grid):
    red = (marked[..., 0] > 170) & (marked[..., 1] < 90) & (marked[..., 2] < 90)
    # The user's boxes are thin outlines, the bars thick: an opening keeps the bars
    labels, count = ndimage.label(ndimage.binary_opening(red, iterations=2))
    bars = []
    for index in range(1, count + 1):
        rows, columns = np.nonzero(labels == index)
        if len(columns) < 40:
            continue
        points = to_grid(np.column_stack([columns, rows]).astype(float))
        centre = points.mean(axis=0)
        # The bar's long axis (principal component): its direction and length
        direction = np.linalg.eigh(np.cov((points - centre).T))[1][:, 1]
        along = (points - centre) @ direction
        angle = np.degrees(np.arctan2(direction[1], direction[0])) % 180
        bars.append((centre[1], centre[0], np.ptp(along), angle))
    print("  grid (x, y)          length  angle (0 = along x)")
    for y_sq, x_sq, length, angle in sorted(bars):
        print(f"  ({x_sq:7.3f}, {y_sq:7.3f})   {length:5.2f}   {angle:5.1f}")


if __name__ == "__main__":
    main()
