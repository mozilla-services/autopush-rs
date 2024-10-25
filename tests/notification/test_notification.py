import time
import logging

from PIL import ImageGrab

import imgcompare
import pytest
from selenium.webdriver.common.by import By


@pytest.fixture
def images_dir(tmpdir):
    return tmpdir.mkdir("images")

@pytest.fixture(autouse=True)
def setup_page(selenium, images_dir):
    selenium.get("localhost:8201")
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    time.sleep(5)  # wait a bit to take the base screenshot
    base_img = ImageGrab.grab()
    base_img.save(f"{images_dir}/base_screenshot.jpg")
    logging.info(images_dir)
    return base_img


@pytest.mark.nondestructive
def test_basic_notification_by_itself(selenium, setup_page, images_dir):
    el = selenium.find_element(By.CSS_SELECTOR,
        ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR,
            "button.popup-notification-primary-button"
        )
        button.click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(setup_page, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_with_altered_title(selenium, images_dir):
    title_box = selenium.find_element(By.CSS_SELECTOR, "#msg_txt")
    title_box.send_keys(" testing titles")
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    base_img = ImageGrab.grab()
    base_img.save(f"{images_dir}/base_screenshot_with_altered_title.jpg")
    el = selenium.find_element(By.CSS_SELECTOR,
        ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR,
            "button.popup-notification-primary-button"
        )
        button.click()
    selenium.find_element(By.CSS_SELECTOR, ".container").click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(base_img, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_with_altered_body(selenium, images_dir):
    body_box = selenium.find_element(By.CSS_SELECTOR, "#body_txt")
    body_box.send_keys(" testing body text")
    base_img = ImageGrab.grab()
    el = selenium.find_element(By.CSS_SELECTOR,
        ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR,
            "button.popup-notification-primary-button"
        )
        button.click()
    base_img.save(f"{images_dir}/base_screenshot_with_altered_body.jpg")
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot_with_altered_body.jpg")
    diff = imgcompare.image_diff_percent(base_img, img)
    assert diff < 2


@pytest.mark.nondestructive
def test_basic_notification_close(selenium, setup_page, images_dir):
    el = selenium.find_element(By.CSS_SELECTOR,
        ".container > p:nth-child(5) > button:nth-child(1)"
    )
    el.click()
    # click allow notification
    with selenium.context(selenium.CONTEXT_CHROME):
        button = selenium.find_element(By.CSS_SELECTOR,
            "button.popup-notification-primary-button"
        )
        button.click()
    img = ImageGrab.grab()
    img.save(f"{images_dir}/screenshot.jpg")
    # compare images
    diff = imgcompare.image_diff_percent(setup_page, img)
    assert diff < 2
    selenium.find_element(By.CSS_SELECTOR, ".container > p:nth-child(6) > button:nth-child(1)").click()
    closed_notification_img = ImageGrab.grab()
    closed_notification_img.save(f"{images_dir}/screenshot_close.jpg")
    diff = imgcompare.image_diff_percent(setup_page, closed_notification_img)
    assert round(diff, 2) <= 0.1  # assert closed page is less than 1% diff from base
