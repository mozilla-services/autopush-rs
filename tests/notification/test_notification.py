"""Module containing the Notification test files for autopush-rs."""

import logging
import time
from typing import Any

import imgcompare
import pytest
from PIL import ImageGrab
from selenium.webdriver.common.by import By


@pytest.fixture
def images_dir(tmpdir: Any) -> Any:
    """Directory to store the screenshots for testing."""
    return tmpdir.mkdir("images")


@pytest.fixture(autouse=True)
def setup_page(selenium: Any, images_dir: str) -> ImageGrab:
    """Fixture to setup the test page and take the base screenshot."""
    selenium.get("localhost:8201")
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    time.sleep(5)  # wait a bit to take the base screenshot
    base_img = ImageGrab.grab()
    base_img.save(f"{images_dir}/base_screenshot.jpg")
    logging.info(images_dir)
    return base_img


@pytest.mark.nondestructive
def test_basic_notification_by_itself(
    selenium: Any, images_dir: str, setup_page: ImageGrab
) -> None:
    """Tests a basic notification with no changes."""
    el = selenium.find_element(
        By.CSS_SELECTOR, ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR, "button.popup-notification-primary-button")
        button.click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(setup_page, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_with_altered_title(selenium: Any, images_dir: str):
    """Tests a basic notification with a different title."""
    title_box = selenium.find_element(By.CSS_SELECTOR, "#msg_txt")
    title_box.send_keys(" testing titles")
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    base_img = ImageGrab.grab()
    base_img.save(f"{images_dir}/base_screenshot_with_altered_title.jpg")
    el = selenium.find_element(
        By.CSS_SELECTOR, ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR, "button.popup-notification-primary-button")
        button.click()
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(base_img, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_with_altered_body(selenium: Any, images_dir: str):
    """Tests a basic notification with an altered notification body."""
    body_box = selenium.find_element(By.CSS_SELECTOR, "#body_txt")
    body_box.send_keys(" testing body text")
    base_img = ImageGrab.grab()
    el = selenium.find_element(
        By.CSS_SELECTOR, ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR, "button.popup-notification-primary-button")
        button.click()
    base_img.save(f"{images_dir}/base_screenshot_with_altered_body.jpg")
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot_with_altered_body.jpg")
    diff = imgcompare.image_diff_percent(base_img, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_close(selenium: Any, images_dir: str, setup_page: ImageGrab):
    """Tests a basic notification with and then closes it."""
    el = selenium.find_element(
        By.CSS_SELECTOR, ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR, "button.popup-notification-primary-button")
        button.click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(setup_page, img)
    assert diff < 2
    selenium.find_element(
        By.CSS_SELECTOR, ".container > p:nth-child(6) > button:nth-child(1)"
    ).click()
    closed_notification_img = ImageGrab.grab()
    closed_notification_img.save(f"{images_dir}/screenshot_close.jpg")
    diff = imgcompare.image_diff_percent(setup_page, closed_notification_img)
    assert round(diff, 2) <= 0.1  # assert closed page is less than 1% diff from base
